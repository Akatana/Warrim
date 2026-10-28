//! Creatures that fight for the player: a pet that follows him, a corpse raised to fight again, a creature taken over for a while (mind control, enslave).
//! They are creatures of the world like the rest (the model, the animations, the name over the head), and the game moves them here instead of by their script:
//! they follow the player, go for what fights him (or what he has selected), strike it, and some cast a spell at it from afar.

use super::*;

/// How far from the player an ally stays when it has nothing to fight, and how far it may be before it comes back to him at once, yards.
const STAY_NEAR: f32 = 3.5;
const COME_BACK: f32 = 45.0;
/// How far an ally goes for what fights the player, yards from him; and how far a spell of an ally reaches.
pub const GO_FOR: f32 = 30.0;
/// A pet within this many yards of a creature that strikes the player takes this share of its blows (in percent).
pub const SOAK_REACH: f32 = 6.0;
pub const SOAK_PERCENT: u32 = 40;
const SPELL_REACH: f32 = 20.0;
/// Seconds before a pet may begin another spell when the last one had no cast time (the global cooldown).
const GLOBAL_COOLDOWN: f32 = 1.5;
/// A creature that is no pet of the original's tables has this much mana, and this much more for each level of its master, and this spirit.
const MANA_BASE: f32 = 40.0;
const MANA_PER_LEVEL: f32 = 14.0;
const SPIRIT: f32 = 20.0;
/// A pet's mana comes back every 2 seconds by its spirit (`Creature::RegeneratePower`: spirit / 5 + 17), unless it spent mana in the last 5 seconds.
const REGEN_EVERY: f32 = 2.0;
const REGEN_AFTER: f32 = 5.0;
/// Seconds a dead ally lies before it is gone (a pet lies until it is revived).
pub const CORPSE_TIME: f32 = 4.0;

/// How far from the player a pet on the aggressive stance goes for a foe that has not touched him, yards.
const AGGRESSIVE_REACH: f32 = 15.0;

/// What a pet does with foes it is not told to attack: nothing, the ones that fight the player, or any about.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Stance {
    Passive,
    #[default]
    Defensive,
    Aggressive,
}

/// What the pet bar has told the pet: its stance, whether it stays where it is, and the creature (its place) it is sent to attack.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PetOrders {
    pub stance: Stance,
    pub stay: bool,
    pub attack: Option<usize>,
}

/// A button of the pet bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PetCommand {
    Attack,
    Follow,
    Stay,
    Stance(Stance),
    /// The spell in this place of the pet bar's spells (`special::PET_BAR_SPELLS`).
    Spell(usize),
}

impl PetOrders {
    /// How far from the player a foe of the stance's own choosing may be, and whether it must be fighting him: `None` for a stance that picks none.
    pub fn pick(&self) -> Option<(f32, bool)> {
        match self.stance {
            Stance::Passive => None,
            Stance::Defensive => Some((GO_FOR, true)),
            Stance::Aggressive => Some((AGGRESSIVE_REACH, false)),
        }
    }
}

/// What an ally is, besides a creature.
#[derive(Clone, Debug)]
pub struct Ally {
    /// Seconds it stays yet (0: until it dies, or is sent away).
    pub left: f32,
    /// The spell of the original it casts at what it fights from afar (0: it only strikes), and seconds before it may cast again (counted from the cast going off).
    pub spell: u32,
    pub cool: f32,
    /// The mana it has and the most it can have (the original's table for its kind and the level of its master), its spirit, and the seconds since it spent
    /// mana and since its mana last came back. Without the mana for its spell it fights close.
    pub mana: f32,
    pub mana_max: f32,
    pub spirit: f32,
    pub spent: f32,
    pub tick: f32,
    /// The level of its master: the level its spells are cast at; and the most health it can have (the original's table for its kind and that level).
    pub level: u32,
    pub health_max: f32,
    /// It is out of phase with the world (Phase Shift): nothing strikes it, until it attacks.
    pub phased: bool,
    /// The spells of the pet that are not ready, and the seconds before they are.
    pub cooldowns: Vec<(u32, f32)>,
    /// Its blows and spells are this many times as hard as its kind's.
    pub strength: f32,
    /// It is a pet: it lies dead until it is revived (any other ally is gone a moment after it dies).
    pub pet: bool,
    /// It is a hostile creature taken over for a while: it is itself again when the time is up.
    pub charmed: bool,
}

impl Mob {
    /// Whether the player can fight it: a hostile or a neutral creature that does not fight for him.
    pub fn attackable(&self) -> bool {
        self.ally.is_none() && (self.def.attackable() || self.forced_hostile)
    }

    /// Whether it fights for the player.
    pub fn is_ally(&self) -> bool {
        self.ally.is_some()
    }

    /// Whether it is a pet (of the player's).
    pub fn is_pet(&self) -> bool {
        self.ally.as_ref().is_some_and(|a| a.pet)
    }

    /// How hard an ally's blow is: its kind's melee attack (or, for a kind that does not melee attack, what its level is worth) times its strength.
    fn ally_blow(&self, ally: &Ally) -> f32 {
        let base = if self.def.melee_damage > 0.0 { self.def.melee_damage } else { 2.0 + self.def.level as f32 * 1.5 };
        base * ally.strength
    }

    /// One step of an ally's life, with the player at `player` and what it is to fight (the creature's place) if there is something: it goes for it and strikes, or follows.
    /// A pet that is told to `stay` keeps its place: it strikes what comes within its reach, and does not follow.
    pub(super) fn step_ally(&mut self, dt: f32, player: Vec3, focus: Option<Vec3>, stay: bool, _ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool, nav: &NavMesh) -> Vec<Act> {
        let Some(mut ally) = self.ally.take() else { return Vec::new() };
        // A cast broken off (the pet was hurt, or died, in the making) ends its glow.
        let mut acts: Vec<Act> = self.out.drain(..).filter(|a| matches!(a, Act::Stop)).collect();
        self.moving = false;
        self.pause = (self.pause - dt).max(0.0);
        ally.cool = (ally.cool - dt).max(0.0);
        // Gone already: it waits to be taken out of the world.
        if self.state == State::Unborn {
            self.ally = Some(ally);
            return acts;
        }
        if let State::Dead(t) = self.state {
            self.state = State::Dead(t + dt);
            // Whoever is not a pet is gone a moment after it died.
            if !ally.pet && t + dt >= CORPSE_TIME {
                self.state = State::Unborn;
            }
            self.ally = Some(ally);
            return acts;
        }
        if ally.left > 0.0 {
            ally.left -= dt;
            if ally.left <= 0.0 {
                if ally.charmed {
                    // It is itself again, and goes home.
                    self.state = State::Return(0.0);
                    return acts;
                }
                self.state = State::Unborn;
                self.ally = Some(ally);
                return acts;
            }
        }
        ally.cooldowns.retain_mut(|c| {
            c.1 -= dt;
            c.1 > 0.0
        });
        // Mana comes back every two seconds, but not within five seconds of spending some.
        (ally.spent, ally.tick) = (ally.spent + dt, ally.tick + dt);
        while ally.tick >= REGEN_EVERY {
            ally.tick -= REGEN_EVERY;
            if ally.spent >= REGEN_AFTER {
                ally.mana = (ally.mana + ally.spirit / 5.0 + 17.0).min(ally.mana_max);
            }
        }
        let spell = content::classic_spell(ally.spell);
        let cost = spell.map_or(0.0, |s| s.mana_cost(ally.mana_max));
        let reach = spell.map_or(SPELL_REACH, Self::reach);
        let to_player = flat(player - self.pos);
        if !stay && to_player.length() > COME_BACK {
            // Left far behind (the player went through a door or rode away): it is where he is.
            self.pos = player + Vec3::new(1.5, 0.0, 1.5);
        }
        match (self.state, focus) {
            // A spell is in the making (the ready stance, the glows), goes off, and the cast animation plays for a moment.
            (State::Cast(_, _, _, false), _) if focus.is_none() => {
                acts.push(Act::Stop);
                self.state = State::Chase;
            }
            (State::Cast(spell, t, total, fired), _) => {
                if let (false, Some(at)) = (fired, focus) {
                    self.turn(flat(at - self.pos));
                }
                let t = t + dt;
                if !fired && t >= total {
                    acts.push(Act::Spell(spell));
                    ally.cool = if total > 0.0 { 0.0 } else { GLOBAL_COOLDOWN };
                }
                self.state = if t >= total + CAST_AFTER { State::Chase } else { State::Cast(spell, t, total, fired || t >= total) };
            }
            (State::Melee(t, hit), _) => {
                let t = t + dt;
                let landed = hit || t >= MELEE_HIT;
                if landed && !hit {
                    ally.phased = false;
                    acts.push(Act::Strike(self.ally_blow(&ally)));
                    self.sounds.push(Cry::CustomAttack);
                }
                self.state = if t >= MELEE_TIME {
                    self.pause = (self.swing() - MELEE_TIME).max(0.1);
                    State::Chase
                } else {
                    State::Melee(t, landed)
                };
            }
            (_, Some(at)) => {
                let to = flat(at - self.pos);
                let dist = to.length();
                self.turn(to);
                let ranged = ally.spell != 0 && ally.mana >= cost;
                // Nothing is cast or struck through a wall: it works round to where it sees its foe.
                let seen = !blocked(self.pos + Vec3::Y * EYE, at + Vec3::Y * EYE);
                if ranged && ally.cool == 0.0 && dist <= reach && seen {
                    // A spell is begun where it stands (one without a cast time goes off at once).
                    let seconds = content::classic_spell(ally.spell).map_or(0.0, |s| s.cast as f32 / 1000.0);
                    (ally.mana, ally.spent) = (ally.mana - cost, 0.0);
                    self.state = State::Cast(ally.spell, 0.0, seconds, false);
                    ally.phased = false;
                    acts.push(Act::Begin(ally.spell));
                    self.sounds.push(Cry::Attack);
                } else if !ranged && dist <= MELEE_RANGE && self.pause == 0.0 && seen {
                    self.state = State::Melee(0.0, false);
                    self.sounds.push(Cry::Attack);
                } else {
                    // A fighter goes up to striking range, a caster stands off (but closes in while a wall hides its foe).
                    let stand = if ranged && seen { reach * 0.7 } else { MELEE_RANGE * 0.8 };
                    if dist > stand && !stay {
                        let step = self.chase_to(at, stand, RUN_SPEED, dt, false, &[], nav, &blocked);
                        self.stepped(step);
                    }
                    self.state = if stay { State::Idle } else { State::Chase };
                }
            }
            (_, None) => {
                let dist = to_player.length();
                if !stay && dist > STAY_NEAR * 1.6 {
                    self.turn(to_player);
                    let step = self.chase_to(player, STAY_NEAR, RUN_SPEED, dt, false, &[], nav, &blocked);
                    self.stepped(step);
                    self.state = State::Chase;
                } else {
                    self.state = State::Idle;
                }
            }
        }
        self.ally = Some(ally);
        acts
    }

    /// It is made a pet or a helper of the player's: its health and the strength of its blows follow the player's level, and it fights until `left` seconds are over (0: for good).
    pub fn make_ally(&mut self, level: u32, left: f32, spell: u32, pet: bool, charmed: bool) {
        let (strength, health_max, mana_max, spirit) = self.ally_stats(level);
        self.ally = Some(Ally { left, spell, cool: 0.0, mana: mana_max, mana_max, spirit, spent: REGEN_AFTER, tick: 0.0, level, health_max, phased: false, cooldowns: Vec::new(), strength, pet, charmed });
        // A creature taken over keeps the health it has; one that is called comes whole, and as strong as the level of the player makes it.
        if !charmed {
            self.health = health_max;
        }
        self.state = State::Idle;
        self.respawn = f32::MAX;
        self.fought = false;
        self.fresh = false;
        self.pause = 0.0;
        (self.stun, self.fear, self.root, self.asleep) = (0.0, 0.0, 0.0, false);
    }

    /// What a master of `level` makes of its kind: the strength of its blows, the most health, the most mana and the spirit. A pet of the original's tables has what they
    /// give (`pet_levelstats`); another creature has its kind's health made as strong as the level asks.
    fn ally_stats(&self, level: u32) -> (f32, f32, f32, f32) {
        let strength = (level as f32 / self.def.level.max(1) as f32).clamp(1.0, 6.0);
        match content::pet_stats(&self.def.id, level) {
            Some((health, mana, _, spirit)) => (strength, health as f32, mana as f32, spirit as f32),
            None => (strength, self.def.health * strength, MANA_BASE + MANA_PER_LEVEL * level as f32, SPIRIT),
        }
    }

    /// The level of the ally's master.
    pub fn ally_level(&self) -> Option<u32> {
        self.ally.as_ref().map(|a| a.level)
    }

    /// The master went up a level: the pet has what the original gives a pet of that level, and (alive) is whole again, as after any level of the original's pets.
    pub fn set_ally_level(&mut self, level: u32) {
        let (strength, health_max, mana_max, spirit) = self.ally_stats(level);
        let alive = !self.is_dead();
        let Some(a) = &mut self.ally else { return };
        (a.level, a.strength, a.health_max, a.mana_max, a.spirit) = (level, strength, health_max, mana_max, spirit);
        if alive {
            (a.mana, self.health) = (mana_max, health_max);
        }
    }

    /// An ally comes back to life with its health whole (a pet revived, a corpse raised).
    pub fn revive_ally(&mut self) {
        self.health = self.ally_max() * 0.5;
        self.state = State::Idle;
        self.skinned = false;
        self.loot = None;
    }

    /// The most health it can have (an ally has more than its kind: see `make_ally`).
    pub fn ally_max(&self) -> f32 {
        self.ally.as_ref().map_or(self.def.health, |a| a.health_max)
    }

    /// Whether it is out of phase with the world.
    pub fn is_phased(&self) -> bool {
        self.ally.as_ref().is_some_and(|a| a.phased)
    }

    /// An ally is healed by `amount`, up to what it can have.
    pub fn mend(&mut self, amount: f32) {
        if !self.is_dead() {
            self.health = (self.health + amount).min(self.ally_max());
        }
    }
}
