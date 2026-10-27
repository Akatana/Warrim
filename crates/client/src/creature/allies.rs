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
/// Seconds between two spells of an ally.
const SPELL_EVERY: f32 = 3.5;
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
    /// The spell of the original it casts at what it fights from afar (0: it only strikes), and seconds before it may cast again.
    pub spell: u32,
    pub cool: f32,
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
    pub(super) fn step_ally(&mut self, dt: f32, player: Vec3, focus: Option<Vec3>, stay: bool, blocked: impl Fn(Vec3, Vec3) -> bool) -> Vec<Act> {
        let Some(mut ally) = self.ally.take() else { return Vec::new() };
        let mut acts = Vec::new();
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
        let to_player = flat(player - self.pos);
        if !stay && to_player.length() > COME_BACK {
            // Left far behind (the player went through a door or rode away): it is where he is.
            self.pos = player + Vec3::new(1.5, 0.0, 1.5);
        }
        match (self.state, focus) {
            (State::Melee(t, hit), _) => {
                let t = t + dt;
                let landed = hit || t >= MELEE_HIT;
                if landed && !hit {
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
                let ranged = ally.spell != 0;
                if ranged && ally.cool == 0.0 && dist <= SPELL_REACH {
                    // A spell goes off from where it stands.
                    acts.push(Act::Spell(ally.spell));
                    ally.cool = SPELL_EVERY;
                    self.state = State::Melee(0.0, true);
                    self.sounds.push(Cry::Attack);
                } else if !ranged && dist <= MELEE_RANGE && self.pause == 0.0 {
                    self.state = State::Melee(0.0, false);
                    self.sounds.push(Cry::Attack);
                } else {
                    // A fighter goes up to striking range, a caster stands off.
                    let stand = if ranged { SPELL_REACH * 0.7 } else { MELEE_RANGE * 0.8 };
                    if dist > stand && !stay {
                        let step = self.walk(to / dist * (RUN_SPEED * dt).min(dist - stand), &blocked);
                        self.stepped(step);
                    }
                    self.state = if stay { State::Idle } else { State::Chase };
                }
            }
            (_, None) => {
                let dist = to_player.length();
                if !stay && dist > STAY_NEAR * 1.6 {
                    self.turn(to_player);
                    let step = self.walk(to_player / dist * (RUN_SPEED * dt).min(dist - STAY_NEAR), &blocked);
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
        let strength = (level as f32 / self.def.level.max(1) as f32).clamp(1.0, 6.0);
        self.ally = Some(Ally { left, spell, cool: 0.0, strength, pet, charmed });
        // A creature taken over keeps the health it has; one that is called comes whole, and as strong as the level of the player makes it.
        if !charmed {
            self.health = self.def.health * strength;
        }
        self.state = State::Idle;
        self.respawn = f32::MAX;
        self.fought = false;
        self.fresh = false;
        self.pause = 0.0;
        (self.stun, self.fear, self.root, self.asleep) = (0.0, 0.0, 0.0, false);
    }

    /// An ally comes back to life with its health whole (a pet revived, a corpse raised).
    pub fn revive_ally(&mut self) {
        let strength = self.ally.as_ref().map_or(1.0, |a| a.strength);
        self.health = self.def.health * strength * 0.5;
        self.state = State::Idle;
        self.skinned = false;
        self.loot = None;
    }

    /// The most health it can have (an ally has more than its kind: see `make_ally`).
    pub fn ally_max(&self) -> f32 {
        self.def.health * self.ally.as_ref().map_or(1.0, |a| a.strength)
    }

    /// An ally is healed by `amount`, up to what it can have.
    pub fn mend(&mut self, amount: f32) {
        if !self.is_dead() {
            self.health = (self.health + amount).min(self.ally_max());
        }
    }
}
