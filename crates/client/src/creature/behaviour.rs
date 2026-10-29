//! One step of a creature's life: aggro, chase, melee attack, roam, return, and which animation it plays.

use super::*;
use crate::navmesh::PATHFIND_NOPATH;

impl Mob {
    /// (Tests) Advance `dt` seconds against a plain player standing at `target`; returns the damage of the swing that landed, if one did.
    #[cfg(test)]
    pub(super) fn update(&mut self, dt: f32, target: Vec3, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool) -> Option<f32> {
        self.update_on(dt, target, ground, blocked, &NavMesh::default())
    }

    /// (Tests) The same on a navigation mesh.
    #[cfg(test)]
    pub(super) fn update_on(&mut self, dt: f32, target: Vec3, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool, nav: &NavMesh) -> Option<f32> {
        self.step(dt, &Watch::at(target), 0, &[], &[], ground, blocked, nav).into_iter().find_map(|a| if let Act::Melee(d) = a { Some(d) } else { None })
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
    pub fn step(&mut self, dt: f32, watch: &Watch, me: usize, roster: &[Friend], crowd: &[Vec3], ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool, nav: &NavMesh) -> Vec<Act> {
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
        self.regenerate(dt);
        if self.fear <= 0.0 {
            self.panic = false;
        }
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
        // (`blocked` is a walker's check and lets a hill pass: the sight line is the terrain's too.)
        self.seen = (dist, !hunting || (!blocked(self.pos + eye, target + eye) && (watch.sight)(self.pos + eye, target + eye)));
        let los = self.seen.1;
        self.near.clear();
        if self.social && hunting {
            let faction = self.def.brain.faction;
            self.near = roster.iter().filter(|f| f.index != me && f.faction == faction).map(|f| Friend { dist: flat(f.pos - self.pos).length(), ..f.clone() }).filter(|f| f.dist <= FRIENDS_RADIUS).collect();
            // It counts among its friends, for the heals it casts on itself.
            self.near.push(Friend { index: me, pos: self.pos, faction, dist: 0.0, health: self.health, max: self.def.health, auras: self.aura_ids() });
        }
        match self.state {
            // In terror it runs from the player to places round him, whatever it was doing.
            _ if self.fear > 0.0 && !self.is_dead() && self.state != State::Unborn => {
                let from = if self.panic { self.pos } else { target };
                let step = self.run_in_terror(from, dt, nav, &blocked);
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
                if self.def.is_hostile() && !self.def.brain.calm && (target - self.pos).length() < noticed && (watch.sight)(self.pos + eye, target + eye) {
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
                                let pace = if self.run { self.run_speed } else { self.walk_speed };
                                // The way to the place is made when it sets out: a patrol's by a path to the place, a stroll's by a ray over the mesh.
                                if !self.planned {
                                    self.planned = true;
                                    if self.roam.is_none() && !self.route.is_empty() {
                                        self.patrol_course(goal, nav, &blocked);
                                    } else if !self.random_course(goal, nav) {
                                        self.halt();
                                    }
                                }
                                let step = self.follow(dt, pace, &blocked);
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
                // `UnitAI::UpdateAI`: a caster that keeps close or at a distance leaves ranged mode when the player is within a blow's reach (it fights him close), and is in it
                // again once he is three yards further than that.
                let mode_now = self.rt.as_ref().and_then(|r| r.mode);
                if mode_now.is_some_and(|m| matches!(m.0, brain::PROXIMITY | brain::DISTANCER)) {
                    let on = self.rt.as_ref().is_some_and(|r| r.ranged_on);
                    if on && self.can_reach(dist, PLAYER_REACH, watch.moving) {
                        self.rt.as_mut().expect("checked").ranged_on = false;
                    } else if !on && !self.can_reach(dist - 3.0, PLAYER_REACH, watch.moving) && self.ranged_ok() {
                        self.rt.as_mut().expect("checked").ranged_on = true;
                    }
                }
                let rt = self.rt.as_ref().expect("the script is only out of the creature while it plays");
                let (melee, moves, ranged, mode) = (rt.melee, rt.combat_move, rt.ranged_on, rt.mode);
                let no_melee = ranged && mode.is_some_and(|m| m.0 == brain::NO_MELEE);
                // The fight is left (`EnterEvadeMode`): the player is led too far once the pursuit time has run out, or the target cannot be got at for too long.
                if self.leash_broken(dt, target) | self.target_unreachable(dt) {
                    self.state = State::Return(0.0);
                } else if melee && !no_melee && self.can_reach(dist, PLAYER_REACH, watch.moving) && self.level_with(target) && self.pause == 0.0 {
                    self.state = State::Melee(0.0, false);
                    self.sounds.push(Cry::Attack);
                } else if moves {
                    // A caster in ranged mode stands at its distance as long as it sees the player; any other goes up to its target and stands at half the reach of a blow from it.
                    let stand = match mode {
                        Some((_, d)) if ranged && los => d.max(self.combined_reach(PLAYER_REACH, true)),
                        _ => self.combined_reach(PLAYER_REACH, true) * CHASE_STAND,
                    };
                    let speed = if self.charge > 0.0 { CHARGE_SPEED } else { self.run_speed };
                    let step = self.chase_to(target, stand, speed, dt, true, crowd, nav, &blocked);
                    self.stepped(step);
                    if step == 0.0 {
                        self.turn(to);
                    }
                }
            }
            // It goes home at once, at a run (`EnterEvadeMode`, `HomeMovementGenerator`), and heals on the way as it does out of combat.
            State::Return(_) => {
                let left = flat(self.home - self.pos);
                if left.length() < 0.3 {
                    (self.state, self.rest) = (State::Idle, self.new_rest());
                } else if self.go_home(dt, nav, &blocked) {
                    self.pos.x = self.home.x;
                    self.pos.z = self.home.z;
                }
            }
            State::Melee(t, hit) => {
                self.turn(to);
                let t = t + dt;
                let landed = hit || t >= MELEE_HIT;
                if landed && !hit && self.can_reach(dist, PLAYER_REACH, watch.moving) && self.level_with(target) {
                    let damage = self.def.melee_damage * self.boost() * (1.0 + self.def.melee_spread * (2.0 * self.random() - 1.0));
                    if damage > 0.0 {
                        self.out.push(Act::Melee(damage));
                        self.refresh_fight();
                    }
                    self.sounds.push(Cry::CustomAttack);
                    self.proc_on_hit();
                }
                if t >= MELEE_TIME {
                    self.pause = (self.swing() - MELEE_TIME).max(0.1);
                    self.state = State::Chase;
                } else {
                    self.state = State::Melee(t, landed);
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
                    // (The way to the friend is made when it sets out.)
                    if t <= dt + 1e-6 {
                        self.plan(goal, false, true, nav, &blocked);
                        self.loose = false;
                    }
                    let step = self.follow(dt, self.run_speed, &blocked);
                    self.stepped(step);
                    self.state = if step == 0.0 { State::Chase } else { State::Flee(t, goal, friend) };
                }
            }
            State::Kite(t) => {
                let want = self.rt.as_ref().and_then(|r| r.mode).map_or(10.0, |m| m.1);
                // It runs from the target to a place `want` yards from it (`DistanceYourself`), once.
                if t == 0.0 && !self.distance_yourself(target, want, nav, &blocked) {
                    self.kite = KITE_PAUSE;
                    self.state = State::Chase;
                } else {
                    let step = self.follow(dt, self.run_speed, &blocked);
                    self.stepped(step);
                    if t + dt >= KITE_TIME || self.course.is_empty() || dist >= want * 0.75 {
                        self.kite = KITE_PAUSE;
                        self.state = State::Chase;
                        self.halt();
                        self.chase = Chase::new();
                    } else {
                        self.state = State::Kite(t + dt);
                    }
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
        // It stops where it stands, and the next chase begins afresh.
        if matches!(self.state, State::Idle | State::Unborn | State::Dead(_) | State::Cast(..)) {
            (self.course, self.planned) = (Vec::new(), false);
        }
        if !self.state.fights() {
            self.chase = Chase::new();
        }
        if self.pos.y == 0.0 {
            if let Some(y) = ground(self.pos.x, self.pos.z, self.pos.y) {
                self.settle(y);
            }
        } else if self.moving || !self.placed {
            // (Also once more the step after it stops: the mesh the course was drawn on lies up to a yard under a steep ramp, and it must not stay buried where it stopped.)
            self.placed = !self.moving;
            // The mesh lies up to a voxel over the terrain: a walker stands on the floor under it (`UpdateAllowedPositionZ`), not on the bed of a lake it swims over.
            self.stand(&ground);
        }
        std::mem::take(&mut self.out)
    }

    /// It stands on the floor it was on: where two floors lie over each other (a mine's tunnel and the planks over it) the course, which the mesh made, and the floors under it
    /// may name different ones, and the creature must not jump from one to the other through what is between. So it stays on the floor within a yard above (the step the player's own
    /// controller makes) or 0.8 yd below the one it stood on (a ramp climbs and falls by less in a step); only where there is none (the course left it in the rock over a mine, it was put somewhere
    /// else) it stands on the highest floor within a yard above its feet, however far below that is. A swimmer keeps its height, or it would sink to the bed.
    pub(super) fn stand(&mut self, ground: &impl Fn(f32, f32, f32) -> Option<f32>) {
        if (self.layer - self.pos.y).abs() > 3.0 {
            self.layer = self.pos.y;
        }
        // The floor level with the one it stood on first (within half a yard over it: a ramp near the terrain over it is not ported up to the terrain); a step up of a yard only where
        // there is none (a plank, a ledge).
        let level = ground(self.pos.x, self.pos.z, self.layer - 0.5).filter(|f| *f > self.layer - 0.8);
        let kept = level.or_else(|| ground(self.pos.x, self.pos.z, self.layer).filter(|f| *f > self.layer - 0.8));
        let y = kept.or_else(|| ground(self.pos.x, self.pos.z, self.pos.y + 1.0).filter(|y| !self.swimming || *y > self.pos.y - 1.0));
        if let Some(y) = y {
            self.settle(y);
        }
    }

    /// The places still to walk through, the next last (for the probes).
    pub fn course_points(&self) -> Vec<Vec3> {
        self.course.clone()
    }

    /// The heights of the places still to walk through (for the probes).
    pub fn course_heights(&self) -> Vec<f32> {
        self.course.iter().map(|p| (p.y * 100.0).round() / 100.0).collect()
    }

    /// A creature that has not been put anywhere yet (height 0) stands on the ground; after that the ways it walks give its height, corrected to the terrain.
    pub(super) fn settle(&mut self, y: f32) {
        (self.pos.y, self.layer) = (y, y);
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
        // In water it swims (a model without the swimming animations goes on as it was).
        if self.swimming && matches!(self.state, State::Idle | State::Wander(_) | State::Return(_) | State::Chase | State::Flee(..) | State::Kite(_)) {
            return if self.moving { SWIM } else { SWIM_IDLE };
        }
        match self.state {
            State::Idle => self.emote.unwrap_or(STAND),
            State::Unborn => STAND,
            State::Return(_) => RUN,
            State::Wander(_) if self.run => RUN,
            State::Wander(_) => WALK,
            State::Chase if self.moving && matches!(self.chase.mode, Mode::Backpedal | Mode::Fanning) => WALK,
            State::Chase if self.moving => RUN,
            State::Chase => STAND,
            State::Flee(..) | State::Kite(_) => RUN,
            State::Melee(..) => ATTACK_UNARMED,
            State::Cast(spell, _, _, fired) => {
                let (ready, cast) = self.spell_anims(spell, anims);
                if fired { cast } else { ready }
            }
            State::Flinch(_) => WOUND,
            State::Dead(_) => DEATH,
        }
    }
}
