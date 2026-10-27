//! A creature casting what its script orders: starting, releasing and finishing a spell, fleeing, fights.

use super::*;

impl Mob {
    /// What the script asks of the creature.
    pub(super) fn order(&mut self, o: Order, near: &[Friend]) -> Outcome {
        match o {
            Order::Cast { spell, aim, flags, friend } => self.start_cast(spell, aim, flags, friend, near),
            Order::Summon { entry, ms } => {
                self.out.push(Act::Summon { entry, ms });
                Outcome::Done
            }
            Order::Flee => self.flee(near),
            Order::Help(radius) => {
                self.out.push(Act::Assist(if radius > 0.0 { radius } else { self.def.brain.assist }));
                Outcome::Done
            }
            Order::Evade => {
                if self.state.fights() {
                    self.state = State::Return(0.0);
                }
                Outcome::Done
            }
            Order::Die | Order::Despawn => {
                if !self.is_dead() {
                    self.health = self.health.min(0.0);
                    self.die();
                }
                Outcome::Done
            }
            Order::RemoveAura(spell) => {
                self.auras.retain(|a| a.0 != spell);
                Outcome::Done
            }
        }
    }

    /// Cast a spell as the script asks: it must be free to cast, the target in reach and in sight, and the mana there; then the cast begins (an instant
    /// spell goes off at once).
    pub(super) fn start_cast(&mut self, spell: u32, aim: Aim, flags: u32, friend: Option<usize>, near: &[Friend]) -> Outcome {
        let Some(sp) = content::classic_spell(spell) else { return Outcome::Done };
        let dead = self.is_dead();
        let triggered = flags & cast::TRIGGERED != 0;
        if !dead && !triggered && !matches!(self.state, State::Idle | State::Wander(_) | State::Chase) {
            // A cast that interrupts breaks off the one in the making; any other waits.
            let Some(State::Cast(_, _, _, fired)) = (flags & cast::INTERRUPT != 0).then_some(self.state) else { return Outcome::Busy };
            if !fired {
                self.out.push(Act::Stop);
            }
            (self.pending, self.state) = (None, State::Chase);
        }
        let aim = if flags & cast::SELF_TARGET != 0 { Aim::Itself } else { aim };
        if flags & cast::AURA_NOT_PRESENT != 0 && aim == Aim::Itself && self.has_aura(spell) {
            return Outcome::Done;
        }
        // A caster that keeps its distance does not cast its main spell at a player who stands next to it: it backs away first.
        let close = self.stance.is_some_and(|m| matches!(m.0, brain::PROXIMITY | brain::DISTANCER)) && self.seen.0 < KITE_RANGE && self.kite == 0.0 && self.ranged_ok();
        if !dead && !triggered && flags & cast::MAIN != 0 && aim == Aim::Victim && close && matches!(self.state, State::Chase) {
            self.state = State::Kite(0.0);
            return Outcome::Busy;
        }
        if !dead && !triggered {
            let (dist, los) = self.seen;
            match aim {
                Aim::Victim if dist > Self::reach(sp) || !los => return Outcome::Cannot,
                Aim::Friend => {
                    let Some(f) = friend.and_then(|i| near.iter().find(|f| f.index == i)) else { return Outcome::Done };
                    if f.dist > Self::reach(sp) {
                        return Outcome::Cannot;
                    }
                }
                _ => {}
            }
            let cost = sp.mana_cost(self.def.brain.mana);
            if self.def.brain.mana > 0.0 && flags & cast::FORCE == 0 && self.mana < cost {
                // The main spell it cannot pay for ends its ranged mode: it fights close.
                self.oom |= flags & cast::MAIN != 0;
                return Outcome::Cannot;
            }
            self.mana = (self.mana - cost).max(0.0);
        }
        if dead {
            self.release(sp, aim, friend);
            return Outcome::Done;
        }
        let seconds = if triggered { 0.0 } else { sp.cast as f32 / 1000.0 };
        if seconds > 0.0 {
            self.pending = Some(Pending { aim, friend });
            self.state = State::Cast(spell, 0.0, seconds, false);
            self.out.push(Act::Begin(spell));
            self.sounds.push(Cry::Attack);
        } else {
            self.release(sp, aim, friend);
            // A spell cast at someone holds the creature in its cast animation for a moment.
            if self.state.fights() && !triggered && aim != Aim::Itself {
                self.state = State::Cast(spell, 0.0, 0.0, true);
                self.pending = None;
            }
        }
        Outcome::Done
    }

    /// The spell goes off.
    pub(super) fn release(&mut self, sp: &'static ClassicSpell, aim: Aim, friend: Option<usize>) {
        if matches!(aim, Aim::Itself | Aim::Around) {
            self.apply(sp);
        }
        let boost = self.boost();
        self.out.push(Act::Cast(Cast { spell: sp.spell, aim, friend, from: self.pos, yaw: self.yaw, level: self.def.level, melee: self.def.melee_damage * boost, shot: self.def.brain.shot * boost }));
        if sp.effects.iter().any(|e| e.kind == 96) {
            self.charge = CHARGE_TIME;
        }
    }

    /// The cast bar is full: the spell goes off if its target is still in reach and in sight.
    pub(super) fn finish_cast(&mut self, spell: u32) {
        let (Some(sp), Some(p)) = (content::classic_spell(spell), self.pending.take()) else { return };
        let (dist, los) = self.seen;
        if p.aim == Aim::Victim && (dist > Self::reach(sp) + 2.0 || !los) {
            self.out.push(Act::Stop);
        } else {
            self.release(sp, p.aim, p.friend);
        }
    }

    /// Run to the nearest friend to call it to help (or run away, if there is none): `Flee for assist`.
    pub(super) fn flee(&mut self, near: &[Friend]) -> Outcome {
        if !self.state.fights() || matches!(self.state, State::Flee(..)) {
            return Outcome::Done;
        }
        if matches!(self.state, State::Cast(_, _, _, false)) {
            self.out.push(Act::Stop);
            self.pending = None;
        }
        let friend = near.iter().filter(|f| f.index != self.me && f.dist <= FLEE_RADIUS && f.dist > 1.0).min_by(|a, b| a.dist.total_cmp(&b.dist));
        self.state = match friend {
            Some(f) => State::Flee(0.0, f.pos, true),
            None => State::Flee(0.0, self.pos + flat(self.pos - self.player).normalize_or_zero() * FLEE_RADIUS, false),
        };
        Outcome::Done
    }

    /// A fight begins: the creature calls the others of its faction, and the events of its script start from their first times.
    pub(super) fn begin_fight(&mut self) {
        if self.def.brain.assist > 0.0 && self.def.brain.faction != 0 {
            self.out.push(Act::Assist(self.def.brain.assist));
        }
        let def = self.def;
        let Some(mut rt) = self.rt.take() else { return };
        let near = self.near.clone();
        rt.enter_combat(&def.brain, &mut |o| self.order(o, &near));
        self.rt = Some(rt);
    }

    /// The fight is over: the script starts afresh, the auras are gone.
    pub(super) fn end_fight(&mut self) {
        let def = self.def;
        if let Some(rt) = &mut self.rt {
            rt.reset(&def.brain);
        }
        self.auras.clear();
        self.pending = None;
        self.charge = 0.0;
        self.oom = false;
        // What its waypoint scripts did does not outlast a fight.
        (self.run, self.stopped, self.roam) = (false, false, None);
        self.todo.clear();
    }

    /// Turn on the player when a friend calls for help (a creature that is not already fighting).
    pub fn assist(&mut self) -> bool {
        let free = matches!(self.state, State::Idle | State::Wander(_) | State::Return(_));
        if free && self.def.attackable() && !self.def.brain.calm {
            self.state = State::Chase;
            self.sounds.push(Cry::Aggro);
            return true;
        }
        false
    }
}
