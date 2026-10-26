//! One step of a creature's life: aggro, chase, bite, roam, return, and which animation it plays.

use super::*;

impl Mob {
    /// (Tests) Advance `dt` seconds against a plain player standing at `target`; returns the damage of the swing that landed, if one did.
    #[cfg(test)]
    pub(super) fn update(&mut self, dt: f32, target: Vec3, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool) -> Option<f32> {
        self.step(dt, &Watch::at(target), 0, &[], ground, blocked).into_iter().find_map(|a| if let Act::Melee(d) = a { Some(d) } else { None })
    }

    /// A summoned creature is put into the world at `pos`, ready to fight, for `ms` milliseconds (0: until it dies or the fight is over).
    pub fn summon_at(&mut self, pos: Vec3, yaw: f32, ms: i32) {
        (self.pos, self.home, self.yaw) = (pos, pos, yaw);
        (self.health, self.mana, self.loot) = (self.def.health, self.def.brain.mana, None);
        self.life = ms.max(0) as f32 / 1000.0;
        self.state = State::Chase;
        self.fought = false;
        self.fresh = true;
        self.end_fight();
    }

    /// Advance `dt` seconds with the player as `watch` has him, on ground at `ground` where nothing `blocked` is in the way; `me` is its place among the
    /// creatures and `roster` the creatures that are about. Returns what it did.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, dt: f32, watch: &Watch, me: usize, roster: &[Friend], ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool) -> Vec<Act> {
        let Some(dt) = self.afflicted(dt) else { return Vec::new() };
        let target = watch.feet;
        self.pause = (self.pause - dt).max(0.0);
        self.kite = (self.kite - dt).max(0.0);
        self.charge = (self.charge - dt).max(0.0);
        self.auras.retain_mut(|a| {
            a.1 -= dt;
            a.1 > 0.0
        });
        self.moving = false;
        (self.me, self.player) = (me, target);
        if !self.todo.is_empty() && !self.is_dead() {
            let mut due = Vec::new();
            self.todo.retain_mut(|t| {
                t.0 -= dt;
                if t.0 <= 0.0 {
                    due.push(t.1.clone());
                }
                t.0 > 0.0
            });
            for step in due {
                self.do_step(&step);
            }
        }
        let to = flat(target - self.pos);
        let dist = to.length();
        // The way to the player: what a spell needs to be cast at him.
        let hunting = self.state.fights();
        let eye = Vec3::Y * EYE;
        self.seen = (dist, !hunting || !blocked(self.pos + eye, target + eye));
        let los = self.seen.1;
        self.near.clear();
        if self.social && hunting {
            let faction = self.def.brain.faction;
            self.near = roster.iter().filter(|f| f.index != me && f.faction == faction).map(|f| Friend { dist: flat(f.pos - self.pos).length(), ..f.clone() }).filter(|f| f.dist <= FRIENDS_RADIUS).collect();
            // It counts among its friends, for the heals it casts on itself.
            self.near.push(Friend { index: me, pos: self.pos, faction, dist: 0.0, health: self.health, max: self.def.health, auras: self.aura_ids() });
        }
        match self.state {
            // In terror it runs from the player, whatever it was doing.
            _ if self.fear > 0.0 && !self.is_dead() && self.state != State::Unborn => {
                let away = if dist > 0.01 { -to / dist } else { Vec3::Z };
                self.turn(away);
                let step = self.walk(away * RUN_SPEED * dt, &blocked);
                self.stepped(step);
                if !self.state.fights() {
                    self.state = State::Chase;
                }
            }
            State::Unborn => {}
            State::Dead(_) => self.tick_dead(dt),
            State::Flinch(t) => self.state = if t + dt >= FLINCH_TIME { State::Chase } else { State::Flinch(t + dt) },
            State::Idle | State::Wander(_) => {
                let noticed = aggro_range(self.def.level, watch.level) * if self.hidden { SNEAK_AGGRO * self.notice } else { 1.0 };
                if self.def.is_hostile() && !self.def.brain.calm && (target - self.pos).length() < noticed {
                    self.state = State::Chase;
                    self.sounds.push(Cry::Aggro);
                } else if self.attackable() || !self.route.is_empty() {
                    match self.state {
                        State::Wander(goal) => {
                            let left = flat(goal - self.pos);
                            if left.length() < 0.3 && self.roam.is_none() && !self.route.is_empty() {
                                self.arrive();
                            } else if left.length() < 0.3 {
                                self.legs = self.legs.saturating_sub(1);
                                if self.legs > 0 {
                                    self.start_leg();
                                } else {
                                    (self.state, self.rest) = (State::Idle, self.new_rest());
                                }
                            } else {
                                self.yaw = left.x.atan2(-left.z);
                                let pace = if self.run { RUN_SPEED } else { WALK_SPEED };
                                let step = self.walk(left.normalize() * (pace * dt).min(left.length()), &blocked);
                                if step == 0.0 {
                                    // Something is in the way: it gives up the stroll.
                                    (self.state, self.rest, self.legs) = (State::Idle, self.new_rest(), 0);
                                }
                                self.stride += step;
                                if self.stride >= STEP * 0.6 {
                                    self.stride = 0.0;
                                    self.sounds.push(Cry::Footstep);
                                }
                            }
                        }
                        _ => {
                            self.rest -= dt;
                            if let Some((centre, radius, left)) = self.roam {
                                // A script has it stroll round here for a while, and then the patrol goes on.
                                if left <= dt {
                                    (self.roam, self.rest) = (None, 0.0);
                                } else {
                                    self.roam = Some((centre, radius, left - dt));
                                    if self.rest <= 0.0 {
                                        self.legs = 1 + (self.random() * 3.0) as u32;
                                        self.start_leg();
                                    }
                                }
                            } else if self.rest <= 0.0 && !self.route.is_empty() {
                                if !self.stopped {
                                    self.state = State::Wander(self.route[self.next].at);
                                }
                            } else if self.rest <= 0.0 && self.wander > 0.0 {
                                self.legs = 1 + (self.random() * 3.0) as u32;
                                self.start_leg();
                            }
                        }
                    }
                }
            }
            State::Chase => {
                self.turn(to);
                let rt = self.rt.as_ref().expect("the script is only out of the creature while it plays");
                let (melee, moves, ranged, mode) = (rt.melee, rt.combat_move, rt.ranged_on, rt.mode);
                let no_melee = ranged && mode.is_some_and(|m| m.0 == brain::NO_MELEE);
                if dist > LEASH_RANGE || flat(self.home - self.pos).length() > LEASH_RANGE {
                    self.state = State::Return(0.0);
                } else if melee && !no_melee && dist <= BITE_RANGE && self.pause == 0.0 {
                    self.state = State::Bite(0.0, false);
                    self.sounds.push(Cry::Attack);
                } else if ranged && self.kite == 0.0 && dist < KITE_RANGE && mode.is_some_and(|m| matches!(m.0, brain::PROXIMITY | brain::DISTANCER)) && self.ranged_ok() {
                    // A caster backs away from a player who comes close.
                    self.state = State::Kite(0.0);
                } else if moves {
                    // A caster in ranged mode stands at its distance as long as it sees the player; any other runs to him.
                    let stand = match mode {
                        Some((_, d)) if ranged && los => d.max(BITE_RANGE),
                        _ => BITE_RANGE * 0.8,
                    };
                    if dist > stand {
                        let speed = if self.charge > 0.0 { CHARGE_SPEED } else { RUN_SPEED };
                        let step = self.walk(to / dist * (speed * dt).min(dist - stand), &blocked);
                        self.stepped(step);
                    }
                }
            }
            State::Return(t) if t < RETURN_WAIT => self.state = State::Return(t + dt),
            State::Return(_) => {
                let left = flat(self.home - self.pos);
                if left.length() < 0.3 {
                    (self.state, self.health, self.rest) = (State::Idle, self.def.health, self.new_rest());
                    self.mana = self.def.brain.mana;
                } else {
                    self.yaw = left.x.atan2(-left.z);
                    if self.walk(left.normalize() * (RUN_SPEED * dt).min(left.length()), &blocked) == 0.0 {
                        // Stuck behind something: it does not stay out there wounded forever.
                        (self.pos.x, self.pos.z) = (self.home.x, self.home.z);
                    }
                    self.health = (self.health + self.def.health * REGEN * dt).min(self.def.health);
                }
            }
            State::Bite(t, hit) => {
                self.turn(to);
                let t = t + dt;
                let landed = hit || t >= BITE_HIT;
                if landed && !hit && dist <= BITE_REACH {
                    let damage = self.def.bite_damage * self.boost();
                    if damage > 0.0 {
                        self.out.push(Act::Melee(damage));
                    }
                    self.sounds.push(Cry::Bite);
                    self.proc_on_hit();
                }
                if t >= BITE_TIME {
                    self.pause = (self.swing() - BITE_TIME).max(0.1);
                    self.state = State::Chase;
                } else {
                    self.state = State::Bite(t, landed);
                }
            }
            State::Cast(spell, t, total, fired) => {
                if self.pending.is_none_or(|p| p.aim == Aim::Victim) {
                    self.turn(to);
                }
                let t = t + dt;
                if !fired && t >= total {
                    self.finish_cast(spell);
                }
                self.state = if t >= total + CAST_AFTER {
                    if self.fought { State::Chase } else { State::Idle }
                } else {
                    State::Cast(spell, t, total, fired || t >= total)
                };
            }
            State::Flee(t, goal, friend) => {
                let t = t + dt;
                let left = flat(goal - self.pos);
                if t >= FLEE_TIME || (friend && left.length() < 2.0) {
                    // At the friend (or out of breath): it calls for help and goes back to the fight.
                    if friend && self.def.brain.faction != 0 {
                        self.out.push(Act::Assist(self.def.brain.assist.max(10.0)));
                    }
                    self.state = State::Chase;
                } else {
                    self.turn(left);
                    let step = self.walk(left.normalize_or_zero() * (RUN_SPEED * dt).min(left.length()), &blocked);
                    self.stepped(step);
                    self.state = if step == 0.0 { State::Chase } else { State::Flee(t, goal, friend) };
                }
            }
            State::Kite(t) => {
                let away = if dist > 0.01 { -to / dist } else { Vec3::Z };
                self.turn(away);
                let want = self.rt.as_ref().and_then(|r| r.mode).map_or(10.0, |m| m.1);
                let step = self.walk(away * RUN_SPEED * dt, &blocked);
                self.stepped(step);
                if t + dt >= KITE_TIME || step == 0.0 || dist >= want * 0.75 {
                    self.kite = KITE_PAUSE;
                    self.state = State::Chase;
                } else {
                    self.state = State::Kite(t + dt);
                }
            }
        }
        // A summoned creature goes when its time is up or the fight is over.
        if self.summoned && !self.is_dead() {
            let expired = self.life > 0.0 && {
                self.life -= dt;
                self.life <= 0.0
            };
            if expired || matches!(self.state, State::Idle | State::Wander(_) | State::Return(_)) {
                if matches!(self.state, State::Cast(_, _, _, false)) {
                    self.out.push(Act::Stop);
                }
                self.state = State::Unborn;
            }
        }
        // A fight begins or ends.
        let now = self.state.fights();
        if now && !self.fought {
            self.begin_fight();
        } else if !now && self.fought && !self.is_dead() {
            self.end_fight();
        }
        self.fought = now;
        if self.fresh && !self.is_dead() {
            self.fresh = false;
            self.fire(brain::event::SPAWNED);
        }
        self.think(dt, watch);
        if let Some(y) = ground(self.pos.x, self.pos.z, self.pos.y) {
            self.pos.y = y;
        }
        std::mem::take(&mut self.out)
    }

    /// Play the script for `dt` seconds.
    pub(super) fn think(&mut self, dt: f32, watch: &Watch) {
        let def = self.def;
        if def.brain.events.is_empty() || self.is_dead() {
            return;
        }
        let Some(mut rt) = self.rt.take() else { return };
        rt.set_ranged(!self.oom);
        self.stance = rt.mode.filter(|_| rt.ranged_on);
        let near = self.near.clone();
        let auras = self.aura_ids();
        let forward = Vec3::new(watch.yaw.sin(), 0.0, -watch.yaw.cos());
        let behind = flat(self.pos - watch.feet).normalize_or_zero().dot(forward) < 0.0;
        let mana = def.brain.mana;
        let view = View {
            in_combat: self.state.fights(),
            health: self.health / def.health,
            mana: if mana > 0.0 { self.mana / mana } else { 1.0 },
            has_mana: mana > 0.0,
            auras: &auras,
            dist: self.seen.0,
            player_health: watch.health,
            player_mana: watch.mana,
            player_casting: watch.casting,
            player_auras: watch.auras,
            behind,
            friends: &near,
        };
        rt.think(&def.brain, dt, &view, &mut |o| self.order(o, &near));
        self.rt = Some(rt);
    }

    /// The animation that draws its bow, fires its gun or throws, by the ranged weapon it holds (a person's outfit has it); the cast of a spell for any other.
    pub(super) fn shot_anim(&self) -> u16 {
        let Body::Humanoid { outfit, .. } = &self.def.body else { return CAST_SPELL };
        match outfit.iter().map(|&i| items::get(i)).find(|i| matches!(i.wear, items::Wear::Ranged | items::Wear::Thrown)).map(|i| (i.class, i.subclass)) {
            Some((2, 2 | 18)) => crate::animator::ATTACK_BOW,
            Some((2, 3)) => crate::animator::ATTACK_RIFLE,
            Some((2, 16)) => crate::animator::ATTACK_1H,
            _ => CAST_SPELL,
        }
    }

    /// The stance held while `spell` is cast and the cast itself: what the spell's own kit names, or the draw of the weapon for a ranged attack (Shoot and
    /// Throw have no kit: the original gets theirs from the weapon), or the ones of any spell.
    pub(super) fn spell_anims(&self, spell: u32, anims: &SpellAnims) -> (u16, u16) {
        let (ready, cast) = anims.get(&spell).copied().unwrap_or_default();
        let shot = content::classic_spell(spell).is_some_and(|s| s.is_ranged_attack()) && cast.is_none();
        (ready.unwrap_or(if shot { self.shot_anim() } else { READY_SPELL }), cast.unwrap_or(if shot { self.shot_anim() } else { CAST_SPELL }))
    }

    /// The animation the creature plays; `anims` are the ones its spells ask for (the stance held while casting, the cast).
    pub(super) fn animation(&self, anims: &SpellAnims) -> u16 {
        match self.state {
            State::Idle => self.emote.unwrap_or(STAND),
            State::Unborn => STAND,
            State::Return(t) if t < RETURN_WAIT => STAND,
            State::Return(_) => RUN,
            State::Wander(_) if self.run => RUN,
            State::Wander(_) => WALK,
            State::Chase if self.moving => RUN,
            State::Chase => STAND,
            State::Flee(..) | State::Kite(_) => RUN,
            State::Bite(..) => BITE,
            State::Cast(spell, _, _, fired) => {
                let (ready, cast) = self.spell_anims(spell, anims);
                if fired { cast } else { ready }
            }
            State::Flinch(_) => WOUND,
            State::Dead(_) => DEATH,
        }
    }
}
