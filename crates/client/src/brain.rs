//! What a creature does in a fight beyond biting, as the original's server decides it: the scripts of its EventAI (`content/classic_creature_ai.ron`, made by
//! `tools/import_creature_ai.py` from the CMaNGOS database). A script is a list of events (a timer in combat, health falling to some percent, the player being in
//! some range, a friend hurt, an aura missing...) and what each does when it fires (cast a spell at someone, summon, change phase, enable ranged mode, flee and
//! call for help...). `Runtime` plays the script: it counts the timers, tests the events, rolls their chance, runs their actions and re-arms them the way
//! CMaNGOS does (`CreatureEventAI::UpdateEventTimers`, `ProcessEvent`, `ResetEvent`). What the actions do in the world (cast, summon, flee) is asked of the creature
//! through the callback; the actions that only change the script's own state (phases, ranged mode, combat movement, melee) are done here.

use crate::content::{Brain, BrainEvent};
use glam::Vec3;

/// EventAI event types the game plays.
pub mod event {
    pub const TIMER: u8 = 0;
    pub const HEALTH: u8 = 2;
    pub const MANA: u8 = 3;
    pub const AGGRO: u8 = 4;
    pub const DEATH: u8 = 6;
    pub const RANGE: u8 = 9;
    pub const SPAWNED: u8 = 11;
    pub const TARGET_HEALTH: u8 = 12;
    pub const TARGET_CASTING: u8 = 13;
    pub const FRIEND_HEALTH: u8 = 14;
    pub const FRIEND_MISSING_BUFF: u8 = 16;
    pub const TARGET_MANA: u8 = 18;
    pub const AURA: u8 = 23;
    pub const TARGET_AURA: u8 = 24;
    pub const MISSING_AURA: u8 = 27;
    pub const TARGET_MISSING_AURA: u8 = 28;
    pub const GENERIC: u8 = 29;
    pub const ATTACKING_TARGET: u8 = 32;
    pub const FACING: u8 = 33;
}

/// EventAI action types the game plays.
pub mod act {
    pub const CAST: u8 = 11;
    pub const SUMMON: u8 = 12;
    pub const AUTO_ATTACK: u8 = 20;
    pub const COMBAT_MOVEMENT: u8 = 21;
    pub const SET_PHASE: u8 = 22;
    pub const INC_PHASE: u8 = 23;
    pub const EVADE: u8 = 24;
    pub const FLEE: u8 = 25;
    pub const REMOVE_AURA: u8 = 28;
    pub const RANGED_MOVEMENT: u8 = 29;
    pub const RANDOM_PHASE: u8 = 30;
    pub const RANDOM_PHASE_RANGE: u8 = 31;
    pub const DIE: u8 = 37;
    pub const CALL_FOR_HELP: u8 = 39;
    pub const DESPAWN: u8 = 41;
    pub const RANGED_MODE: u8 = 57;
}

/// The flags of an event.
pub mod flag {
    pub const REPEATABLE: u32 = 0x1;
    pub const RANDOM_ACTION: u32 = 0x20;
    pub const RANGED_ONLY: u32 = 0x100;
    pub const MELEE_ONLY: u32 = 0x200;
    /// Only while the creature can act in a fight: when its first action cannot be done (out of range, out of mana) the event waits and tries again.
    pub const COMBAT_ACTION: u32 = 0x400;
}

/// The flags of a cast action.
pub mod cast {
    pub const INTERRUPT: u32 = 0x1;
    pub const TRIGGERED: u32 = 0x2;
    pub const FORCE: u32 = 0x4;
    pub const SELF_TARGET: u32 = 0x10;
    pub const AURA_NOT_PRESENT: u32 = 0x20;
    /// The spell a caster stands back to cast (its ranged mode keeps it in range of this one while it has mana for it).
    pub const MAIN: u32 = 0x100;
}

/// Ranged modes (action 57): a full caster stays at its distance; a proximity caster backs away when the player comes close; one without melee never
/// fights close; a distancer backs away from the player regularly.
pub const FULL_CASTER: u32 = 1;
pub const PROXIMITY: u32 = 2;
pub const NO_MELEE: u32 = 3;
pub const DISTANCER: u32 = 4;

/// Who a cast is aimed at, from the target code of the action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Aim {
    /// The creature itself.
    Itself,
    /// Whoever it fights: the player.
    Victim,
    /// A friend the event picked (the hurt one, the one without the buff).
    Friend,
    /// Everyone around it.
    Around,
}

impl Aim {
    pub fn of(code: i32) -> Aim {
        match code {
            0 => Aim::Itself,
            12 | 13 | 14 => Aim::Friend,
            15 | 17 => Aim::Around,
            _ => Aim::Victim,
        }
    }
}

/// What a script asks the creature to do in the world.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Order {
    Cast { spell: u32, aim: Aim, flags: u32, friend: Option<usize> },
    Summon { entry: u32, ms: i32 },
    Flee,
    Help(f32),
    Evade,
    Die,
    Despawn,
    RemoveAura(u32),
}

/// How an order went: it is done, the creature is busy (it tries again at once), or it cannot (out of range, out of mana: a combat action tries again,
/// another event waits for its next time).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Done,
    Busy,
    Cannot,
}

/// An event of a script in words, for the editor.
pub fn describe(e: &BrainEvent) -> String {
    let secs = |ms: i32| format!("{:.1}", ms as f32 / 1000.0);
    let spell = |id: i32| crate::content::classic_spell(id as u32).map_or(format!("spell {id}"), |s| s.name.clone());
    let when = match e.on {
        event::TIMER => format!("In a fight, after {}-{} s, then every {}-{} s", secs(e.param(0)), secs(e.param(1)), secs(e.param(2)), secs(e.param(3))),
        event::GENERIC => format!("Every {}-{} s", secs(e.param(2)), secs(e.param(3))),
        event::HEALTH => format!("Its health is {}-{}%", e.param(1), e.param(0)),
        event::MANA => format!("Its mana is {}-{}%", e.param(1), e.param(0)),
        event::AGGRO => "A fight begins".into(),
        event::DEATH => "It dies".into(),
        event::SPAWNED => "It appears".into(),
        event::RANGE | event::ATTACKING_TARGET => format!("The player is {}-{} yards away", e.param(0), e.param(1)),
        event::TARGET_HEALTH => format!("The player's health is {}-{}%", e.param(1), e.param(0)),
        event::TARGET_CASTING => "The player casts".into(),
        event::TARGET_MANA => format!("The player's mana is {}-{}%", e.param(1), e.param(0)),
        event::FRIEND_HEALTH => format!("A friend within {} yards is hurt", e.param(1)),
        event::FRIEND_MISSING_BUFF => format!("A friend within {} yards lacks {}", e.param(1), spell(e.param(0))),
        event::AURA => format!("It has the aura of {}", spell(e.param(0))),
        event::MISSING_AURA => format!("It lacks the aura of {}", spell(e.param(0))),
        event::TARGET_AURA => format!("The player has the aura of {}", spell(e.param(0))),
        event::TARGET_MISSING_AURA => format!("The player lacks the aura of {}", spell(e.param(0))),
        event::FACING => if e.param(0) == 0 { "It is behind the player".into() } else { "It is in front of the player".into() },
        other => format!("Event {other}"),
    };
    let does: Vec<String> = e
        .acts
        .iter()
        .map(|&(kind, a, b, _)| match kind {
            act::CAST => format!("cast {} on {}", spell(a), match Aim::of(b) { Aim::Itself => "itself", Aim::Victim => "the player", Aim::Friend => "a friend", Aim::Around => "all around" }),
            act::SUMMON => format!("summon creature {a}"),
            act::FLEE => "run for help".into(),
            act::CALL_FOR_HELP => "call for help".into(),
            act::EVADE => "give up the fight".into(),
            act::DIE => "die".into(),
            act::DESPAWN => "go".into(),
            act::RANGED_MODE => format!("fight from {b} yards (mode {a})"),
            act::RANGED_MOVEMENT => format!("fight from {a} yards"),
            act::SET_PHASE => format!("go to phase {a}"),
            act::INC_PHASE => format!("phase {a:+}"),
            act::AUTO_ATTACK => format!("melee {}", if a != 0 { "on" } else { "off" }),
            act::COMBAT_MOVEMENT => format!("moving {}", if a != 0 { "on" } else { "off" }),
            other => format!("action {other}"),
        })
        .collect();
    format!("{when}: {}", does.join(", "))
}

/// A creature that is about, for the events that look after friends (and for finding who to run to and who to call).
#[derive(Clone, Debug)]
pub struct Friend {
    pub index: usize,
    pub pos: Vec3,
    pub faction: u32,
    /// Yards from the creature that asks.
    pub dist: f32,
    pub health: f32,
    pub max: f32,
    /// The spells whose auras it has.
    pub auras: Vec<u32>,
}

/// What a creature knows when it thinks.
pub struct View<'a> {
    pub in_combat: bool,
    /// Its own health and mana as shares of the most (mana 1 for a creature that has none).
    pub health: f32,
    pub mana: f32,
    pub has_mana: bool,
    pub auras: &'a [u32],
    /// Yards from the player (the one it fights).
    pub dist: f32,
    pub player_health: f32,
    pub player_mana: f32,
    pub player_casting: bool,
    pub player_auras: &'a [u32],
    /// It stands behind the player.
    pub behind: bool,
    pub friends: &'a [Friend],
}

/// The script of a creature being played.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub phase: u32,
    /// Seconds before each event may fire (`None`: it has fired and does not repeat).
    timers: Vec<Option<f32>>,
    /// The ranged mode the script set: its type and the distance to stand at; and whether it is on now.
    pub mode: Option<(u32, f32)>,
    pub ranged_on: bool,
    /// May it move to the player (or stand where it is), may it swing.
    pub combat_move: bool,
    pub melee: bool,
    rng: u32,
}

/// The type of event that fires once (when something happens) and is never disabled by firing.
fn is_trigger(on: u8) -> bool {
    matches!(on, event::AGGRO | event::DEATH | event::SPAWNED)
}

impl Runtime {
    pub fn new(brain: &Brain, seed: u32) -> Runtime {
        let mut r = Runtime { phase: 0, timers: Vec::new(), mode: None, ranged_on: false, combat_move: true, melee: true, rng: seed | 1 };
        r.reset(brain);
        r
    }

    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// A number of seconds between the two times (in milliseconds).
    fn between(&mut self, min: i32, max: i32) -> f32 {
        let (min, max) = (min.max(0) as u32, max.max(min).max(0) as u32);
        (min + self.random() % (max - min + 1)) as f32 / 1000.0
    }

    /// How long until the event first fires: a timer waits a time between its first two numbers, the rest are ready at once.
    fn first(&mut self, ev: &BrainEvent) -> Option<f32> {
        Some(match ev.on {
            event::TIMER | event::GENERIC => self.between(ev.param(0), ev.param(1)),
            _ => 0.0,
        })
    }

    /// How long until it fires again: a time between its repeat numbers.
    fn again(&mut self, ev: &BrainEvent) -> Option<f32> {
        let at = if ev.on == event::TARGET_CASTING { 0 } else { 2 };
        Some(self.between(ev.param(at), ev.param(at + 1)))
    }

    /// Back to the start: the first phase, every event ready as at the start of a fight, melee and no ranged mode.
    pub fn reset(&mut self, brain: &Brain) {
        self.phase = 0;
        self.mode = None;
        self.ranged_on = false;
        (self.combat_move, self.melee) = (true, true);
        self.timers = brain.events.iter().map(|ev| self.first(ev)).collect();
    }

    /// A fight begins: the timers start from their first times again, and the aggro events fire.
    pub fn enter_combat(&mut self, brain: &Brain, run: &mut impl FnMut(Order) -> Outcome) {
        self.timers = brain.events.iter().map(|ev| self.first(ev)).collect();
        self.fire(brain, event::AGGRO, run);
    }

    /// Run every event of this type that its phase allows (the ones that happen at a moment: aggro, spawn, death).
    pub fn fire(&mut self, brain: &Brain, on: u8, run: &mut impl FnMut(Order) -> Outcome) {
        for ev in brain.events.iter().filter(|e| e.on == on) {
            if ev.off & (1 << self.phase.min(31)) != 0 || (ev.chance < 100 && self.random() % 100 >= ev.chance) {
                continue;
            }
            self.run_acts(ev, None, run);
        }
    }

    /// Whether the event's condition holds now; for the events about friends, with the friend they found.
    fn holds(&self, ev: &BrainEvent, v: &View) -> Option<Option<usize>> {
        let between = |share: f32, max: i32, min: i32| {
            let percent = share * 100.0;
            percent <= max as f32 && percent >= min as f32
        };
        let met = match ev.on {
            event::TIMER => v.in_combat,
            event::GENERIC => true,
            event::HEALTH => v.in_combat && between(v.health, ev.param(0), ev.param(1)),
            event::MANA => v.in_combat && v.has_mana && between(v.mana, ev.param(0), ev.param(1)),
            event::RANGE | event::ATTACKING_TARGET => v.in_combat && v.dist >= ev.param(0) as f32 && v.dist <= ev.param(1) as f32,
            event::TARGET_HEALTH => v.in_combat && between(v.player_health, ev.param(0), ev.param(1)),
            event::TARGET_CASTING => v.in_combat && v.player_casting,
            event::TARGET_MANA => v.in_combat && between(v.player_mana, ev.param(0), ev.param(1)),
            event::FRIEND_HEALTH => {
                if !v.in_combat {
                    return None;
                }
                let (deficit, radius, percent) = (ev.param(0) as f32, ev.param(1) as f32, ev.param(4) != 0);
                let hurt = v.friends.iter().filter(|f| f.dist <= radius && f.health < f.max && if percent { (1.0 - f.health / f.max) * 100.0 >= deficit } else { f.max - f.health >= deficit });
                return hurt.min_by(|a, b| (a.health / a.max).total_cmp(&(b.health / b.max))).map(|f| Some(f.index));
            }
            event::FRIEND_MISSING_BUFF => {
                if !v.in_combat {
                    return None;
                }
                let (spell, radius) = (ev.param(0) as u32, ev.param(1) as f32);
                return v.friends.iter().find(|f| f.dist <= radius && !f.auras.contains(&spell)).map(|f| Some(f.index));
            }
            event::AURA => v.auras.contains(&(ev.param(0) as u32)),
            event::MISSING_AURA => !v.auras.contains(&(ev.param(0) as u32)),
            event::TARGET_AURA => v.in_combat && v.player_auras.contains(&(ev.param(0) as u32)),
            event::TARGET_MISSING_AURA => v.in_combat && !v.player_auras.contains(&(ev.param(0) as u32)),
            // Position 0: it stands behind its target; 1: it stands in front.
            event::FACING => v.in_combat && v.behind == (ev.param(0) == 0),
            _ => false,
        };
        met.then_some(None)
    }

    /// Play the script for `dt` seconds.
    pub fn think(&mut self, brain: &Brain, dt: f32, v: &View, run: &mut impl FnMut(Order) -> Outcome) {
        for (i, ev) in brain.events.iter().enumerate() {
            // An event whose phase is not on does not count down.
            if ev.off & (1 << self.phase.min(31)) != 0 || is_trigger(ev.on) {
                continue;
            }
            let Some(left) = self.timers[i] else { continue };
            if left > dt {
                self.timers[i] = Some(left - dt);
                continue;
            }
            self.timers[i] = Some(0.0);
            let Some(friend) = self.holds(ev, v) else { continue };
            if (ev.flags & flag::RANGED_ONLY != 0 && !self.ranged_on) || (ev.flags & flag::MELEE_ONLY != 0 && self.ranged_on) {
                continue;
            }
            if ev.chance < 100 && self.random() % 100 >= ev.chance {
                self.timers[i] = self.again(ev);
                continue;
            }
            match self.run_acts(ev, friend, run) {
                // It did nothing: a combat action waits for another chance, the others lose this one.
                Outcome::Busy => {}
                Outcome::Cannot if ev.flags & flag::COMBAT_ACTION != 0 => {}
                _ => self.timers[i] = if ev.flags & flag::REPEATABLE != 0 { self.again(ev) } else { None },
            }
        }
    }

    /// Do the actions of an event (one of them, chosen by chance, for a random-action event). How the first one went is how the event went.
    fn run_acts(&mut self, ev: &BrainEvent, friend: Option<usize>, run: &mut impl FnMut(Order) -> Outcome) -> Outcome {
        let one;
        let acts: &[(u8, i32, i32, i32)] = if ev.flags & flag::RANDOM_ACTION != 0 && !ev.acts.is_empty() {
            one = [ev.acts[self.random() as usize % ev.acts.len()]];
            &one
        } else {
            &ev.acts
        };
        let mut first = None;
        for &(kind, a, b, c) in acts {
            let outcome = self.act(kind, a, b, c, friend, run);
            let first = first.get_or_insert(outcome);
            if *first != Outcome::Done {
                // The first action failed: the rest are not done.
                break;
            }
        }
        first.unwrap_or(Outcome::Done)
    }

    fn act(&mut self, kind: u8, a: i32, b: i32, c: i32, friend: Option<usize>, run: &mut impl FnMut(Order) -> Outcome) -> Outcome {
        match kind {
            act::CAST => return run(Order::Cast { spell: a as u32, aim: Aim::of(b), flags: c as u32, friend }),
            act::SUMMON => return run(Order::Summon { entry: a as u32, ms: c }),
            act::FLEE => return run(Order::Flee),
            act::CALL_FOR_HELP => return run(Order::Help(a as f32)),
            act::EVADE => return run(Order::Evade),
            act::DIE => return run(Order::Die),
            act::DESPAWN => return run(Order::Despawn),
            act::REMOVE_AURA => return run(Order::RemoveAura(b as u32)),
            act::AUTO_ATTACK => self.melee = a != 0,
            act::COMBAT_MOVEMENT => self.combat_move = a != 0,
            act::SET_PHASE => self.phase = a.clamp(0, 31) as u32,
            act::INC_PHASE => self.phase = (self.phase as i32 + a).clamp(0, 31) as u32,
            act::RANDOM_PHASE => {
                let options: Vec<i32> = [a, b, c].into_iter().filter(|&p| p >= 0).collect();
                self.phase = options[self.random() as usize % options.len()].clamp(0, 31) as u32;
            }
            act::RANDOM_PHASE_RANGE => self.phase = (a + (self.random() % (b - a).max(0) as u32 + 1) as i32).clamp(0, 31) as u32,
            // Ranged mode: 0 turns it off; any other type stands at the distance. (Ranged movement only gives the distance.)
            act::RANGED_MODE => {
                self.mode = (a != 0).then_some((a as u32, b as f32));
                self.ranged_on = a != 0;
            }
            act::RANGED_MOVEMENT => {
                self.mode = (a > 0).then_some((FULL_CASTER, a as f32));
                self.ranged_on = a > 0;
            }
            _ => {}
        }
        Outcome::Done
    }

    /// The ranged mode is turned off (a creature that is out of mana fights close); `on` turns it back on when mana is back.
    pub fn set_ranged(&mut self, on: bool) {
        self.ranged_on = on && self.mode.is_some();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brain(events: Vec<BrainEvent>) -> Brain {
        Brain { events, ..Brain::default() }
    }

    fn event(on: u8, p: &[i32], flags: u32, acts: &[(u8, i32, i32, i32)]) -> BrainEvent {
        BrainEvent { on, p: p.to_vec(), flags, acts: acts.to_vec(), ..BrainEvent::default() }
    }

    fn view() -> View<'static> {
        View { in_combat: true, health: 1.0, mana: 1.0, has_mana: true, auras: &[], dist: 10.0, player_health: 1.0, player_mana: 1.0, player_casting: false, player_auras: &[], behind: false, friends: &[] }
    }

    /// Play `seconds` at 10 steps a second and list the spells cast.
    fn casts(b: &Brain, r: &mut Runtime, seconds: f32, v: &View) -> Vec<u32> {
        let mut out = Vec::new();
        for _ in 0..(seconds * 10.0) as usize {
            r.think(b, 0.1, v, &mut |o| {
                if let Order::Cast { spell, .. } = o {
                    out.push(spell);
                }
                Outcome::Done
            });
        }
        out
    }

    #[test]
    fn a_timer_waits_its_first_time_then_repeats_if_it_may() {
        // First after 2 seconds, then every 3; the second one fires once only.
        let b = brain(vec![event(event::TIMER, &[2000, 2000, 3000, 3000], flag::REPEATABLE, &[(act::CAST, 7, 1, 0)]), event(event::TIMER, &[1000, 1000, 1000, 1000], 0, &[(act::CAST, 8, 1, 0)])]);
        let mut r = Runtime::new(&b, 1);
        assert_eq!(casts(&b, &mut r, 8.5, &view()), [8, 7, 7, 7]);
        // Out of combat a timer in combat does not run.
        let mut r = Runtime::new(&b, 1);
        assert!(casts(&b, &mut r, 5.0, &View { in_combat: false, ..view() }).is_empty());
    }

    #[test]
    fn health_events_fire_in_their_window_and_phases_gate_events() {
        // Flee-like at 15%: once, and a second event that only plays in phase 1.
        let b = brain(vec![
            event(event::HEALTH, &[15], 0, &[(act::CAST, 1, 0, 0), (act::SET_PHASE, 1, 0, 0)]),
            BrainEvent { off: 0b01, ..event(event::TIMER, &[0, 0, 1000, 1000], flag::REPEATABLE, &[(act::CAST, 2, 1, 0)]) },
        ]);
        let mut r = Runtime::new(&b, 1);
        assert!(casts(&b, &mut r, 3.0, &View { health: 0.5, ..view() }).is_empty() || r.phase == 0, "phase 0 blocks the second event");
        let mut r = Runtime::new(&b, 1);
        // The events are played in order: the phase the first one sets is the phase the second one finds.
        assert_eq!(casts(&b, &mut r, 0.1, &View { health: 0.1, ..view() }), [1, 2]);
        assert_eq!(r.phase, 1);
        let later = casts(&b, &mut r, 3.0, &View { health: 0.1, ..view() });
        assert!((2..=3).contains(&later.len()), "{later:?}");
        assert!(later.iter().all(|&s| s == 2), "the first event does not fire twice");
    }

    #[test]
    fn range_events_and_ranged_mode() {
        let b = brain(vec![event(event::AGGRO, &[], 0, &[(act::RANGED_MODE, FULL_CASTER as i32, 25, 0)]), event(event::RANGE, &[5, 30, 2000, 2000], flag::REPEATABLE | flag::COMBAT_ACTION | flag::RANGED_ONLY, &[(act::CAST, 9, 1, cast::MAIN as i32)])]);
        let mut r = Runtime::new(&b, 1);
        assert!(casts(&b, &mut r, 1.0, &view()).is_empty(), "the shot is for ranged mode only");
        r.enter_combat(&b, &mut |_| Outcome::Done);
        assert_eq!((r.mode, r.ranged_on), (Some((FULL_CASTER, 25.0)), true));
        assert_eq!(casts(&b, &mut r, 4.0, &view()).len(), 2);
        assert!(casts(&b, &mut r, 4.0, &View { dist: 3.0, ..view() }).is_empty(), "too close for the shot");
        assert!(casts(&b, &mut r, 4.0, &View { dist: 40.0, ..view() }).is_empty(), "too far for the shot");
        // Out of mana it turns the mode off and the shot stops.
        r.set_ranged(false);
        assert!(casts(&b, &mut r, 4.0, &view()).is_empty());
    }

    #[test]
    fn a_combat_action_that_cannot_be_done_waits_and_tries_again() {
        let b = brain(vec![event(event::RANGE, &[0, 40, 5000, 5000], flag::REPEATABLE | flag::COMBAT_ACTION, &[(act::CAST, 3, 1, 0)]), event(event::RANGE, &[0, 40, 5000, 5000], flag::REPEATABLE, &[(act::CAST, 4, 1, 0)])]);
        let mut r = Runtime::new(&b, 1);
        let mut tried = Vec::new();
        for i in 0..20 {
            r.think(&b, 0.1, &view(), &mut |o| {
                if let Order::Cast { spell, .. } = o {
                    tried.push((i, spell));
                }
                Outcome::Cannot
            });
        }
        // The combat action tries on every step; the plain event gives up its turn and waits five seconds.
        assert_eq!(tried.iter().filter(|t| t.1 == 3).count(), 20);
        assert_eq!(tried.iter().filter(|t| t.1 == 4).count(), 1);
    }

    #[test]
    fn friends_are_looked_after_by_the_one_in_most_need() {
        let b = brain(vec![event(event::FRIEND_HEALTH, &[30, 40, 1000, 1000, 1], flag::REPEATABLE | flag::COMBAT_ACTION, &[(act::CAST, 5, 12, 0)])]);
        let friend = |index, dist, health| Friend { index, pos: Vec3::ZERO, faction: 0, dist, health, max: 100.0, auras: vec![] };
        let friends = vec![friend(2, 10.0, 80.0), friend(3, 12.0, 40.0), friend(4, 50.0, 5.0)];
        let mut r = Runtime::new(&b, 1);
        let mut healed = Vec::new();
        r.think(&b, 0.1, &View { friends: &friends, ..view() }, &mut |o| {
            if let Order::Cast { friend, aim, .. } = o {
                healed.push((friend, aim));
            }
            Outcome::Done
        });
        assert_eq!(healed, [(Some(3), Aim::Friend)], "the one at 40% in range, not the one far away");
    }

    #[test]
    fn auras_and_the_target_decide_events() {
        let b = brain(vec![
            event(event::MISSING_AURA, &[324, 1, 1000, 1000], flag::REPEATABLE, &[(act::CAST, 324, 0, 0)]),
            event(event::TARGET_CASTING, &[1000, 1000], flag::REPEATABLE | flag::COMBAT_ACTION, &[(act::CAST, 1766, 1, 0)]),
            event(event::FACING, &[0, 0, 1000, 1000], flag::REPEATABLE | flag::COMBAT_ACTION, &[(act::CAST, 53, 1, 0)]),
        ]);
        let mut r = Runtime::new(&b, 1);
        assert_eq!(casts(&b, &mut r, 0.5, &view()), [324], "it buffs itself when the buff is missing");
        let mut r = Runtime::new(&b, 1);
        let with_buff = View { auras: &[324], ..view() };
        assert!(casts(&b, &mut r, 3.0, &with_buff).is_empty());
        assert_eq!(casts(&b, &mut r, 0.5, &View { player_casting: true, ..with_buff }), [1766], "it kicks a caster");
        let mut r = Runtime::new(&b, 1);
        assert_eq!(casts(&b, &mut r, 0.5, &View { behind: true, ..with_buff }), [53], "backstab from behind");
    }

    #[test]
    fn an_event_is_told_in_words_for_the_editor() {
        let e = event(event::RANGE, &[5, 30, 2500, 5900], flag::REPEATABLE, &[(act::CAST, 133, 1, 0), (act::SET_PHASE, 2, 0, 0)]);
        assert_eq!(describe(&e), "The player is 5-30 yards away: cast Fireball on the player, go to phase 2");
    }
}
