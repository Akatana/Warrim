//! What happens to a creature: damage, afflictions, auras, blows taken.

use super::*;

impl Mob {
    /// Whether a save has to keep it: it is hurt, dead, away from rest or carries loot (anything else starts afresh).
    pub fn worth_saving(&self) -> bool {
        !matches!(self.state, State::Idle | State::Wander(_)) || self.health < self.def.health || self.loot.is_some()
    }

    /// Dead, or waiting in the pool of summoned creatures: either way it is not in the world.
    pub fn is_dead(&self) -> bool {
        matches!(self.state, State::Dead(_) | State::Unborn)
    }

    /// The player's sword hits for `damage`: it flinches, turns on whoever hit it, or dies. A spell being cast is only held up by it.
    pub fn hurt(&mut self, damage: f32) {
        if self.is_dead() {
            return;
        }
        self.health -= damage;
        self.sounds.push(if self.health <= 0.0 { Cry::Death } else { Cry::Wound });
        if self.health <= 0.0 {
            self.loot = self.def.loot.as_deref().and_then(content::drop_table).map(|table| table.roll(&mut self.rng));
            self.die();
            return;
        }
        self.state = match self.state {
            State::Bite(..) | State::Flee(..) | State::Kite(_) => self.state,
            State::Cast(spell, t, total, fired) => State::Cast(spell, if fired { t } else { (t - PUSHBACK).max(0.0) }, total, fired),
            _ => State::Flinch(0.0),
        };
    }

    /// Something is put on it (see `Status`).
    pub fn afflict(&mut self, status: Status) {
        if self.is_dead() {
            return;
        }
        match status {
            Status::Over { amount, period, seconds } => self.over.push((amount, period, period, seconds)),
            Status::Stun(seconds) => self.stun = self.stun.max(seconds),
            Status::Slow(share, seconds) => self.slow = if self.slow.1 > 0.0 { (self.slow.0.min(share), self.slow.1.max(seconds)) } else { (share, seconds) },
        }
    }

    /// `dt` seconds go by for what is on it: the damage over time, a stun that holds it still (`None`: it does nothing), a slow that makes its time run
    /// slower (the whole of it, not only its feet: a shortcut, as the gear that slows is rare). Returns the seconds it lives.
    pub(super) fn afflicted(&mut self, dt: f32) -> Option<f32> {
        if self.is_dead() {
            (self.over, self.stun, self.slow) = (Vec::new(), 0.0, (1.0, 0.0));
            return Some(dt);
        }
        let mut damage = 0.0;
        for o in &mut self.over {
            o.2 -= dt;
            // A tick that is a hair late for the steps of the frames is still the tick of its second.
            while o.2 <= 1e-3 {
                o.2 += o.1.max(0.1);
                damage += o.0;
            }
            o.3 -= dt;
        }
        self.over.retain(|o| o.3 > 0.0);
        if damage > 0.0 && !self.is_dead() {
            self.health -= damage;
            if self.health <= 0.0 {
                self.sounds.push(Cry::Death);
                self.loot = self.def.loot.as_deref().and_then(content::drop_table).map(|table| table.roll(&mut self.rng));
                self.slain_over = true;
                self.die();
                return Some(dt);
            }
        }
        self.slow.1 = (self.slow.1 - dt).max(0.0);
        if self.stun > 0.0 {
            self.stun = (self.stun - dt).max(0.0);
            return None;
        }
        Some(if self.slow.1 > 0.0 { dt * self.slow.0 } else { dt })
    }

    /// It is dead: what it cast stops, and its death events play.
    pub(super) fn die(&mut self) {
        if matches!(self.state, State::Cast(_, _, _, false)) {
            self.out.push(Act::Stop);
        }
        self.state = State::Dead(0.0);
        self.skinned = false;
        self.pending = None;
        self.fire(brain::event::DEATH);
    }

    /// Play the events of this type of the script (the ones that happen at a moment).
    pub(super) fn fire(&mut self, on: u8) {
        let def = self.def;
        let Some(mut rt) = self.rt.take() else { return };
        let near = self.near.clone();
        rt.fire(&def.brain, on, &mut |o| self.order(o, &near));
        self.rt = Some(rt);
    }

    /// The player's sword swing from `from` facing `yaw` reaches this creature if it is hostile, within `reach` yards
    /// and within 60 degrees of straight ahead (or so close that the angle does not matter): it takes `damage`.
    pub fn struck_by(&mut self, from: Vec3, yaw: f32, reach: f32, damage: f32) -> bool {
        let hit = self.in_swing(from, yaw, reach);
        if hit {
            self.hurt(damage);
        }
        hit
    }

    /// Whether a swing from `from` facing `yaw` reaches this creature (see `struck_by`).
    pub(super) fn in_swing(&self, from: Vec3, yaw: f32, reach: f32) -> bool {
        let to = flat(self.pos - from);
        let ahead = to.length() < 0.5 || to.normalize().dot(Vec3::new(yaw.sin(), 0.0, -yaw.cos())) > 0.5;
        self.def.attackable() && !self.is_dead() && to.length() <= reach && ahead
    }

    /// How close this creature is to noticing a sneaking player at `target` (feet): 0 beyond its normal range, 1 once it has noticed
    /// (a sneaking player is noticed from `SNEAK_AGGRO` of the range, so the way between is what the reticle's eye shows).
    pub fn alert(&self, target: Vec3, level: u32) -> f32 {
        match self.state {
            _ if !self.def.is_hostile() || self.def.brain.calm || self.is_dead() => 0.0,
            State::Return(_) => 0.0,
            State::Idle | State::Wander(_) => ((1.0 - (target - self.pos).length() / aggro_range(self.def.level, level)) / (1.0 - SNEAK_AGGRO * self.notice)).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }

    /// Move by `delta` unless something solid stands in the way (`blocked(from, to)`): then slide along it, one axis at a time, or stay. Returns how
    /// far it went.
    pub(super) fn walk(&mut self, delta: Vec3, blocked: &impl Fn(Vec3, Vec3) -> bool) -> f32 {
        for d in [delta, Vec3::new(delta.x, 0.0, 0.0), Vec3::new(0.0, 0.0, delta.z)] {
            if d.length() > 1e-6 && !blocked(self.pos, self.pos + d) {
                self.pos += d;
                self.moving = true;
                return d.length();
            }
        }
        0.0
    }

    /// Turn to face `to` (flat, from the creature).
    pub(super) fn turn(&mut self, to: Vec3) {
        if to.length() > 0.01 {
            self.yaw = to.x.atan2(-to.z);
        }
    }

    /// Footsteps while running.
    pub(super) fn stepped(&mut self, step: f32) {
        self.stride += step;
        if self.stride >= STEP {
            self.stride -= STEP;
            self.sounds.push(Cry::Footstep);
        }
    }

    /// The seconds between its swings.
    pub(super) fn swing(&self) -> f32 {
        if self.def.brain.swing > 0.0 { self.def.brain.swing } else { 2.0 }
    }

    /// Whether it has the mana to back away from a player and cast from afar (twice what its main spell takes, or no need of mana).
    pub(super) fn ranged_ok(&self) -> bool {
        self.def.brain.mana <= 0.0 || self.main_cost <= 0.0 || self.mana > self.main_cost * 2.0
    }

    /// Whether the aura of this spell is on it.
    pub fn has_aura(&self, spell: u32) -> bool {
        self.auras.iter().any(|a| a.0 == spell)
    }

    pub fn aura_ids(&self) -> Vec<u32> {
        self.auras.iter().map(|a| a.0).collect()
    }

    /// How much harder its blows are for the auras on it (an enrage).
    pub(super) fn boost(&self) -> f32 {
        self.auras
            .iter()
            .filter_map(|a| content::classic_spell(a.0))
            .flat_map(|s| s.effects.iter())
            .filter(|e| e.aura == 79 && e.low + e.high > 0)
            .fold(1.0, |b, e| b + (e.low + e.high) as f32 / 2.0 / 100.0)
    }

    /// What a spell does to the creature when it is for friends and it is one (a heal, an aura).
    pub fn apply(&mut self, sp: &ClassicSpell) {
        for (n, e) in sp.effects.iter().enumerate() {
            if !e.friendly() {
                continue;
            }
            match e.kind {
                10 => {
                    let roll = self.next_u32();
                    self.health = (self.health + sp.points(n, self.def.level, roll)).min(self.def.health);
                }
                67 => self.health = self.def.health,
                6 => {
                    let seconds = if sp.duration == 0 { 3600.0 } else { sp.duration as f32 / 1000.0 };
                    self.auras.retain(|a| a.0 != sp.spell);
                    self.auras.push((sp.spell, seconds));
                }
                _ => {}
            }
        }
    }

    /// Its blow lands: the poisons and other procs on it may go off.
    pub(super) fn proc_on_hit(&mut self) {
        for spell in self.aura_ids() {
            let Some(sp) = content::classic_spell(spell).filter(|s| s.proc.0 & 0x14 != 0) else { continue };
            let chance = if sp.proc.1 == 0 || sp.proc.1 > 100 { 100 } else { sp.proc.1 };
            for e in sp.effects.iter().filter(|e| e.aura == 42 && e.trigger != 0) {
                if self.next_u32() % 100 < chance {
                    if let Some(trigger) = content::classic_spell(e.trigger) {
                        self.release(trigger, Aim::Victim, None);
                    }
                }
            }
        }
    }

    /// How far a spell reaches from the creature: its range, or the reach of a blow.
    pub(super) fn reach(sp: &ClassicSpell) -> f32 {
        sp.range.max(5.0)
    }
}
