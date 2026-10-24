//! Creatures: what stands in the world and is not the player. A hostile beast (a wolf) guards its patch, chases the
//! player once close, bites, flinches when hit, dies and comes back after a while; a person (a guard) stands where
//! he is and can be talked to. Which kinds exist and where they stand is in `content/` (creatures.ron, spawns.ron).
//! `Mob` is the behaviour (no graphics, so it is tested on its own); `Creatures` draws them: beasts with the
//! model's own animations, people as the same dressed human male the player is.

use crate::anim;
use crate::animator::{Channel, Motion, Rng};
use crate::audio::{Cry, Sfx};
use crate::brain::{self, cast, Aim, Friend, Order, Outcome, Runtime, View};
use crate::camera::Camera;
use crate::cursor::Kind;
use crate::character::{Appearance, Character};
use crate::content::{self, Body, ClassicSpell, CreatureDef};
use crate::items::{self, SLOTS};
use crate::worn::Worn;
use crate::loot::Loot;
use crate::quests::Marker;
use crate::models::{ModelCache, ModelCpu, ModelVertex};
use crate::physics::{Bodies, RAGDOLL_REACH};
use crate::ragdoll::Ragdoll;
use crate::renderer::{Gpu, Renderer};
use crate::ui::{Quad, Ui, FULL_UV, WHITE};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wowfmt::m2::M2;

/// Animation ids of the sequences every creature model has.
const STAND: u16 = 0;
const WALK: u16 = 4;
const RUN: u16 = 5;
const DEATH: u16 = 1;
/// How hard (yards a second) a creature is thrown away from the player as it dies.
const RAGDOLL_PUSH: f32 = 3.0;
const WOUND: u16 = 9;
const BITE: u16 = 16;
/// The stance held while a spell is cast at a target, and the cast itself (the animations of `AnimationData.dbc`; a spell's own kit may name others).
const READY_SPELL: u16 = 51;
const CAST_SPELL: u16 = 53;

/// How far in front of the player his sword reaches, yards.
pub const SWORD_REACH: f32 = 3.5;
/// How far from a creature of the same level as the player he is noticed, yards; each level the creature is above him adds a yard (up to 45, at 25 levels
/// above) and each level below takes one away, down to the 5 yards of a blow's reach (MaNGOS `Creature::GetAttackDistance`).
const AGGRO_RANGE: f32 = 20.0;
const AGGRO_LEAST: f32 = 5.0;
/// The most a creature is above the player that counts.
const AGGRO_LEVELS: i32 = 25;
/// A creature notices a sneaking player from this share of that distance.
const SNEAK_AGGRO: f32 = 0.35;
/// A chase is given up when the player is this far from the creature, or the creature this far from home, yards.
const LEASH_RANGE: f32 = 40.0;
/// After giving up, seconds it stands unbothered before it runs home; it heals this share of its health per second on the way.
const RETURN_WAIT: f32 = 5.0;
const REGEN: f32 = 0.25;
/// A bite starts within this distance and lands if the player is still within `BITE_REACH`, yards.
const BITE_RANGE: f32 = 2.4;
const BITE_REACH: f32 = 3.2;
/// Seconds into the 1.5 s bite animation at which the teeth land, and when it is over.
const BITE_HIT: f32 = 0.55;
const BITE_TIME: f32 = 1.5;
/// Seconds between two bites, and of a flinch.
const FLINCH_TIME: f32 = 0.45;
/// Seconds a creature stays in its cast animation after a spell has gone off.
const CAST_AFTER: f32 = 0.5;
/// Seconds that being hit delays a spell being cast (the original's pushback).
const PUSHBACK: f32 = 0.5;
/// A creature that runs for help runs this many seconds if nobody is near to run to (the original's flee delay), and looks for a friend within this many yards.
const FLEE_TIME: f32 = 7.0;
const FLEE_RADIUS: f32 = 30.0;
/// A caster that backs away from a player who comes within this many yards, for at most `KITE_TIME` seconds, and not again within `KITE_PAUSE`.
const KITE_RANGE: f32 = 5.0;
const KITE_TIME: f32 = 1.6;
const KITE_PAUSE: f32 = 6.0;
/// How fast a creature charges (yards per second) and for how long.
const CHARGE_SPEED: f32 = 28.0;
const CHARGE_TIME: f32 = 0.6;
/// Friends of a creature are looked for within this many yards (the widest radius of the scripts).
const FRIENDS_RADIUS: f32 = 40.0;
/// How high above the feet the eyes are, for the line of sight of a spell.
const EYE: f32 = 1.2;
/// Chase speed in yards per second: the run animation's own speed.
const RUN_SPEED: f32 = 6.94;
/// Creatures that have not noticed anyone stroll as CMaNGOS has them: to places within `wander` yards of home (the spawn's own number;
/// this is the database's default), at walking speed (yards a second), one to three walks in a row, and then they stand still for a few
/// seconds (between these two numbers).
const WANDER_DEFAULT: f32 = 5.0;
const WALK_SPEED: f32 = 2.5;
const REST: (f32, f32) = (3.0, 10.0);
/// A paw hits the ground every this many yards of running (two per stride of the gallop).
const STEP: f32 = 2.3;
/// Creatures further than this from the player are not animated or drawn, yards.
const DRAW_RANGE: f32 = 150.0;
/// The world holds tens of thousands of places where creatures stand (`Site`): one within `ACTIVATE` yards of the player becomes a creature (a `Mob`, no model yet;
/// the model is made within `DRAW_RANGE`), and one that is resting beyond `DEACTIVATE` is put away again. Cells of the grid that finds them are `CELL` yards wide.
const ACTIVATE: f32 = 200.0;
const DEACTIVATE: f32 = 260.0;
const CELL: f32 = 128.0;
/// Seconds between two looks at who is near, how many creatures get their model in one frame at most, and the longest a frame spends on that (at least one is made).
const SCAN: f32 = 0.25;
const MAKE_PER_FRAME: usize = 3;
/// Creatures nearer than the first are posed every frame, nearer than the second every other frame, the rest every fourth.
const POSE_NEAR: f32 = 45.0;
const POSE_FAR: f32 = 90.0;
const MAKE_TIME: Duration = Duration::from_millis(12);
/// The most places the compass and the map are told of for a kind of creature (the nearest).
const MOST_LEADS: usize = 40;
/// The models of the quest marks floating over a creature's head, by `Marker` (the original's own: they spin).
const MARKER_MODELS: [&str; 3] = [r"Interface\Buttons\TalkToMe.m2", r"Interface\Buttons\TalkToMeQuestionMark.m2", r"Interface\Buttons\TalkToMeQuestion_Grey.m2"];
/// How high above a person's feet the mark's model stands (its own origin; it reaches 0.15 to 0.77 above that), over the name.
const MARKER_HEIGHT: f32 = 2.65;
/// Names float over creatures within this distance, yards, and sit this high above the feet (beasts, people).
const NAME_RANGE: f32 = 35.0;
const NAME_HEIGHT: (f32, f32) = (1.5, 2.3);

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum State {
    /// Standing still (a hostile creature gets bored of it after a while).
    Idle,
    /// Walking to a place near home, given in render space.
    Wander(Vec3),
    Chase,
    /// Seconds into a bite, and whether its damage has been dealt.
    Bite(f32, bool),
    Flinch(f32),
    /// Gave up the chase: seconds it has stood unbothered, then it runs home healing (hit again, it turns on the attacker).
    Return(f32),
    /// Seconds dead.
    Dead(f32),
    /// Casting a spell (the row of Spell.dbc): seconds into it, the seconds its cast takes, and whether it has gone off.
    Cast(u32, f32, f32, bool),
    /// Runs from the fight to a friend (or away) to call for help: seconds so far, where to, and whether it has a friend to run to.
    Flee(f32, Vec3, bool),
    /// A caster backing away from a player who came too close: seconds so far.
    Kite(f32),
    /// A creature of the pool that summoning uses, not in the world now.
    Unborn,
}

impl State {
    /// Whether a creature in this state is in a fight with the player.
    pub fn fights(self) -> bool {
        matches!(self, State::Chase | State::Bite(..) | State::Flinch(_) | State::Cast(..) | State::Flee(..) | State::Kite(_))
    }
}

/// What is put on a creature: damage over time (the damage of a tick, the seconds between ticks, the seconds it lasts), a stun for some seconds, or a slow
/// that leaves it with a share of its pace for some seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Over { amount: f32, period: f32, seconds: f32 },
    Stun(f32),
    Slow(f32, f32),
}

/// What the player is, to a creature that looks at him: where he is and faces, how well he is, what he is doing and which spells have hold of him.
pub struct Watch<'a> {
    pub feet: Vec3,
    pub yaw: f32,
    /// His level: how far a creature notices him depends on it.
    pub level: u32,
    /// Health and mana as shares of the most.
    pub health: f32,
    pub mana: f32,
    pub casting: bool,
    pub auras: &'a [u32],
    /// Sneaking, and what the skill makes of the distance he is noticed from.
    pub sneaking: Option<f32>,
}

/// The distance a creature of `creature` level notices a player of `player` level from, yards.
pub fn aggro_range(creature: u32, player: u32) -> f32 {
    (AGGRO_RANGE - (player as i32 - creature as i32).max(-AGGRO_LEVELS) as f32).max(AGGRO_LEAST)
}

#[cfg(test)]
impl Watch<'static> {
    /// A player of the first level standing at `feet`, well and doing nothing.
    pub fn at(feet: Vec3) -> Watch<'static> {
        Watch { feet, yaw: 0.0, level: 1, health: 1.0, mana: 1.0, casting: false, auras: &[], sneaking: None }
    }
}

/// A spell that went off, for the game to show and to do to the player.
#[derive(Clone, Debug)]
pub struct Cast {
    pub spell: u32,
    pub aim: Aim,
    /// The friend it was aimed at (an index into the creatures).
    pub friend: Option<usize>,
    pub from: Vec3,
    pub yaw: f32,
    pub level: u32,
    /// The damage of the caster's melee weapon and of its ranged weapon, with what makes them harder.
    pub melee: f32,
    pub shot: f32,
}

/// What a creature did that the rest of the game acts on.
#[derive(Clone, Debug)]
pub enum Act {
    /// A swing landed on the player for this much.
    Melee(f32),
    /// A spell with a cast time began.
    Begin(u32),
    /// The spell being cast stopped without going off.
    Stop,
    /// A spell went off.
    Cast(Cast),
    /// It summons creatures of this entry (for so many milliseconds; 0: until they die).
    Summon { entry: u32, ms: i32 },
    /// It calls the creatures of its faction within so many yards.
    Assist(f32),
}

/// What the game does about a creature's act: the damage of a swing, the start of a spell, its end, a spell gone off (with where on friends it shows).
#[derive(Clone, Debug)]
pub enum Foe {
    Melee(f32),
    Begin { who: usize, spell: u32, at: Vec3, yaw: f32, height: f32 },
    Stop { who: usize },
    Cast { who: usize, cast: Cast, height: f32, allies: Vec<Vec3> },
}

/// A cast in the making: who it is for, and its flags.
#[derive(Clone, Copy, Debug)]
struct Pending {
    aim: Aim,
    friend: Option<usize>,
}

pub struct Mob {
    /// Its kind: an index into the content's creatures (health, bite damage, respawn time, loot).
    kind: usize,
    def: &'static CreatureDef,
    /// Feet position in render space and heading (radians, 0 = north, like the camera's yaw).
    pub pos: Vec3,
    pub yaw: f32,
    home: Vec3,
    pub health: f32,
    pub state: State,
    /// The player is sneaking: it has to come much nearer to be noticed.
    pub hidden: bool,
    /// What the sneaking player's skill makes of the distance he is noticed from (1 for none).
    pub notice: f32,
    pause: f32,
    /// Seconds left of standing still before the next stroll.
    rest: f32,
    /// How far from home it strolls (yards; 0 stands still), and the walks left in the stroll it is on.
    wander: f32,
    legs: u32,
    /// Seconds from its death until it is back (its spawn's own, else its kind's).
    respawn: f32,
    /// A patrol: the places it walks to, round and round, and the one it walks to next. Empty for none.
    route: Vec<Stop>,
    next: usize,
    /// What its waypoint scripts have done: it runs between the places, stands and stays, holds an animation while it stands, and how it looks (a kind).
    run: bool,
    stopped: bool,
    emote: Option<u16>,
    look: usize,
    /// Strolling at random round a place (centre, yards, seconds left) before it goes on with the patrol, and script steps waiting for their time (seconds, what).
    roam: Option<(Vec3, f32, f32)>,
    todo: Vec<(f32, content::Step)>,
    /// What the corpse holds, rolled at its death, until it is looted away or the creature is back.
    pub loot: Option<Loot>,
    rng: u32,
    /// Yards run since the last footstep.
    stride: f32,
    /// Cries made since the last `Creatures::update` took them.
    sounds: Vec<Cry>,
    /// Its script being played (taken out while it plays).
    rt: Option<Runtime>,
    /// Mana it has left (it has all it can when it is not fighting).
    pub mana: f32,
    /// Spells whose auras are on it, and the seconds they last yet.
    auras: Vec<(u32, f32)>,
    /// What the player's spells and the gear's put on it: damage over time (the damage of a tick, seconds between ticks, seconds to the next, seconds left),
    /// seconds stunned, the share of its pace a slow leaves it with and the seconds that lasts, and whether the damage over time killed it.
    over: Vec<(f32, f32, f32, f32)>,
    stun: f32,
    slow: (f32, f32),
    slain_over: bool,
    /// What it did since the last step, for the game.
    out: Vec<Act>,
    /// Whether it fought at the last step (the start and the end of a fight are noticed from the change).
    fought: bool,
    pending: Option<Pending>,
    /// How far the player was, and whether the way to him was clear, as of the last step.
    seen: (f32, bool),
    /// Creatures about, as of the last step: the ones of its faction within reach.
    near: Vec<Friend>,
    /// It ran this step (it runs on the spot in no state of its own).
    moving: bool,
    /// Seconds before it may back away again, and seconds of charge left.
    kite: f32,
    charge: f32,
    /// The mana its main spell takes: a caster stays at range while it has twice that.
    main_cost: f32,
    /// A creature of the summoning pool: seconds left of its summoning (0: until it dies or the fight is over).
    summoned: bool,
    life: f32,
    /// It has just been put into the world or has come back: its spawn events are to be played.
    fresh: bool,
    /// Its script looks after friends (heals, buffs, runs to them): it needs to know who is about.
    social: bool,
    /// Its place among the creatures, and where the player was at the last step.
    me: usize,
    player: Vec3,
    /// The ranged mode it is in (its type and distance), if it is in one now: what a cast looks at.
    stance: Option<(u32, f32)>,
    /// Its main spell failed for want of mana in this fight: it fights close from now on.
    oom: bool,
}

/// The animations the spells of creatures ask for: the stance held while casting and the cast, by the row of `Spell.dbc`.
pub type SpellAnims = std::collections::HashMap<u32, (Option<u16>, Option<u16>)>;

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// A place of a patrol in the world: where (render space), the seconds it stands there, the heading it turns to (radians), and its script.
#[derive(Clone, Debug)]
struct Stop {
    at: Vec3,
    wait: f32,
    facing: Option<f32>,
    steps: Vec<(f32, content::Step)>,
}

/// A creature as saved: everything that makes it this creature in this moment (its kind by id).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MobSave {
    kind: String,
    home: Vec3,
    pos: Vec3,
    yaw: f32,
    health: f32,
    state: State,
    loot: Option<crate::save::LootSave>,
    rng: u32,
    rest: f32,
    pause: f32,
    /// How far along its patrol it is and what its waypoint scripts made of it (a save from before has none of this: it starts its patrol afresh).
    #[serde(default)]
    next: usize,
    #[serde(default)]
    run: bool,
    #[serde(default)]
    stopped: bool,
    #[serde(default)]
    emote: Option<u16>,
    #[serde(default)]
    look: Option<String>,
}

impl Mob {
    pub fn snapshot(&self) -> MobSave {
        MobSave {
            kind: self.def.id.clone(),
            home: self.home,
            pos: self.pos,
            yaw: self.yaw,
            health: self.health,
            state: self.state,
            loot: self.loot.as_ref().map(crate::save::loot_to_save),
            rng: self.rng,
            rest: self.rest,
            pause: self.pause,
            next: self.next,
            run: self.run,
            stopped: self.stopped,
            emote: self.emote,
            look: (self.look != self.kind).then(|| content::get().creatures[self.look].id.clone()),
        }
    }

    /// Put a saved creature back, if it is the same one (the same kind with the same home: the content may have changed since).
    pub fn restore(&mut self, saved: &MobSave) -> bool {
        if saved.kind != self.def.id || flat(saved.home - self.home).length() > 1.0 {
            return false;
        }
        // The height is not taken: it is found again from the spawn's floor (a save made before a spawn knew its floor holds the terrain under a building).
        (self.pos, self.yaw, self.health, self.state, self.rng, self.rest, self.pause) = (Vec3::new(saved.pos.x, self.pos.y, saved.pos.z), saved.yaw, saved.health.min(self.def.health), saved.state, saved.rng, saved.rest, saved.pause);
        // A spell half cast, a run for help or a step back is not kept: it goes on fighting.
        if matches!(self.state, State::Cast(..) | State::Flee(..) | State::Kite(_)) {
            self.state = State::Chase;
        }
        self.loot = saved.loot.as_ref().map(crate::save::loot_from_save);
        // Its patrol goes on from the place it was walking to (if the places are still there).
        if saved.next < self.route.len() {
            self.next = saved.next;
        }
        (self.run, self.stopped, self.emote) = (saved.run, saved.stopped, saved.emote);
        if let Some(i) = saved.look.as_ref().and_then(|id| content::get().creatures.iter().position(|c| c.id == *id)) {
            self.look = i;
        }
        self.fought = false;
        self.fresh = !self.is_dead();
        true
    }

    pub fn new(kind: usize, pos: Vec3, yaw: f32) -> Mob {
        let def = &content::get().creatures[kind];
        let seed = pos.x.to_bits() ^ pos.z.to_bits() ^ 0x9E37_79B9;
        let main = def.brain.events.iter().flat_map(|e| &e.acts).find(|a| a.0 == brain::act::CAST && (a.3 as u32) & cast::MAIN != 0);
        let main_cost = main.and_then(|a| content::classic_spell(a.1 as u32)).map_or(0.0, |s| s.mana_cost(def.brain.mana));
        let social = def.brain.events.iter().any(|e| matches!(e.on, brain::event::FRIEND_HEALTH | brain::event::FRIEND_MISSING_BUFF) || e.acts.iter().any(|a| a.0 == brain::act::FLEE || (a.0 == brain::act::CAST && Aim::of(a.2) == Aim::Friend)));
        let mut mob = Mob {
            kind,
            def,
            pos,
            yaw,
            home: pos,
            health: def.health,
            state: State::Idle,
            hidden: false,
            notice: 1.0,
            pause: 0.0,
            rest: 0.0,
            wander: WANDER_DEFAULT,
            legs: 0,
            respawn: def.respawn,
            route: Vec::new(),
            next: 0,
            run: false,
            stopped: false,
            emote: None,
            look: kind,
            roam: None,
            todo: Vec::new(),
            loot: None,
            rng: seed,
            stride: 0.0,
            sounds: Vec::new(),
            rt: Some(Runtime::new(&def.brain, seed)),
            mana: def.brain.mana,
            auras: Vec::new(),
            over: Vec::new(),
            stun: 0.0,
            slow: (1.0, 0.0),
            slain_over: false,
            out: Vec::new(),
            fought: false,
            pending: None,
            seen: (0.0, true),
            near: Vec::new(),
            moving: false,
            kite: 0.0,
            charge: 0.0,
            main_cost,
            summoned: false,
            life: 0.0,
            fresh: true,
            social,
            me: 0,
            player: pos,
            stance: None,
            oom: false,
        };
        mob.rest = mob.new_rest();
        mob
    }

    /// A number from 0 to 1.
    fn random(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    fn next_u32(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    fn new_rest(&mut self) -> f32 {
        REST.0 + (REST.1 - REST.0) * self.random()
    }

    /// Set out for a place round home: any direction, any distance up to `wander` (so the places cluster near home, as in CMaNGOS).
    fn start_leg(&mut self) {
        // Round home, or round the place where a script told it to stroll.
        let (centre, range) = self.roam.map_or((self.home, self.wander), |r| (r.0, r.1));
        let (angle, radius) = (self.random() * std::f32::consts::TAU, self.random() * range);
        self.state = State::Wander(centre + Vec3::new(angle.sin(), 0.0, angle.cos()) * radius);
    }

    /// At the place of the patrol it was walking to: it turns to the heading the place asks, does what the place's script says (some of it a while after), stands there as
    /// long as the place asks, and goes on to the next.
    fn arrive(&mut self) {
        let stop = self.route[self.next].clone();
        self.next = (self.next + 1) % self.route.len();
        self.state = State::Idle;
        // The original makes a path of one place without a wait stand for five seconds.
        self.rest = if self.route.len() == 1 && stop.wait <= 0.0 { 5.0 } else { stop.wait };
        if let (true, Some(facing)) = (stop.wait > 0.0, stop.facing) {
            self.yaw = facing;
        }
        for (delay, step) in stop.steps {
            if delay <= 0.0 {
                self.do_step(&step);
            } else {
                self.todo.push((delay, step));
            }
        }
    }

    /// Do one thing of a waypoint script.
    fn do_step(&mut self, step: &content::Step) {
        use content::Step;
        match step {
            Step::Run(on) => self.run = *on,
            Step::Roam(radius, seconds) => {
                self.roam = Some((self.pos, *radius, *seconds));
                self.rest = self.new_rest();
            }
            Step::Idle => self.stopped = true,
            Step::Emote(anim) => self.emote = (*anim != 0).then_some(*anim),
            Step::Morph(id) => {
                if let Some(kind) = content::get().creatures.iter().position(|c| c.id == *id) {
                    self.look = kind;
                }
            }
        }
    }

    /// It is back as it was at the start: the patrol from its first place, walking, as it looked.
    fn reset_route(&mut self) {
        (self.next, self.run, self.stopped, self.emote, self.look, self.roam) = (0, false, false, None, self.kind, None);
        self.todo.clear();
    }

    /// A corpse counts down to the day it is back: standing up at home, or (a summoned one) going back to the pool.
    fn tick_dead(&mut self, dt: f32) {
        let State::Dead(t) = self.state else { return };
        if t + dt < self.respawn {
            self.state = State::Dead(t + dt);
        } else if self.summoned {
            self.state = State::Unborn;
        } else {
            (self.pos, self.health, self.loot) = (self.home, self.def.health, None);
            self.reset_route();
            self.mana = self.def.brain.mana;
            self.rest = self.new_rest();
            self.state = State::Idle;
            self.end_fight();
            self.fresh = true;
        }
    }

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
    fn afflicted(&mut self, dt: f32) -> Option<f32> {
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
    fn die(&mut self) {
        if matches!(self.state, State::Cast(_, _, _, false)) {
            self.out.push(Act::Stop);
        }
        self.state = State::Dead(0.0);
        self.pending = None;
        self.fire(brain::event::DEATH);
    }

    /// Play the events of this type of the script (the ones that happen at a moment).
    fn fire(&mut self, on: u8) {
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
    fn in_swing(&self, from: Vec3, yaw: f32, reach: f32) -> bool {
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
    fn walk(&mut self, delta: Vec3, blocked: &impl Fn(Vec3, Vec3) -> bool) -> f32 {
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
    fn turn(&mut self, to: Vec3) {
        if to.length() > 0.01 {
            self.yaw = to.x.atan2(-to.z);
        }
    }

    /// Footsteps while running.
    fn stepped(&mut self, step: f32) {
        self.stride += step;
        if self.stride >= STEP {
            self.stride -= STEP;
            self.sounds.push(Cry::Footstep);
        }
    }

    /// The seconds between its swings.
    fn swing(&self) -> f32 {
        if self.def.brain.swing > 0.0 { self.def.brain.swing } else { 2.0 }
    }

    /// Whether it has the mana to back away from a player and cast from afar (twice what its main spell takes, or no need of mana).
    fn ranged_ok(&self) -> bool {
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
    fn boost(&self) -> f32 {
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
    fn proc_on_hit(&mut self) {
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
    fn reach(sp: &ClassicSpell) -> f32 {
        sp.range.max(5.0)
    }

    /// What the script asks of the creature.
    fn order(&mut self, o: Order, near: &[Friend]) -> Outcome {
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
    fn start_cast(&mut self, spell: u32, aim: Aim, flags: u32, friend: Option<usize>, near: &[Friend]) -> Outcome {
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
    fn release(&mut self, sp: &'static ClassicSpell, aim: Aim, friend: Option<usize>) {
        if matches!(aim, Aim::Itself | Aim::Around) {
            self.apply(sp);
        }
        let boost = self.boost();
        self.out.push(Act::Cast(Cast { spell: sp.spell, aim, friend, from: self.pos, yaw: self.yaw, level: self.def.level, melee: self.def.bite_damage * boost, shot: self.def.brain.shot * boost }));
        if sp.effects.iter().any(|e| e.kind == 96) {
            self.charge = CHARGE_TIME;
        }
    }

    /// The cast bar is full: the spell goes off if its target is still in reach and in sight.
    fn finish_cast(&mut self, spell: u32) {
        let (Some(sp), Some(p)) = (content::classic_spell(spell), self.pending.take()) else { return };
        let (dist, los) = self.seen;
        if p.aim == Aim::Victim && (dist > Self::reach(sp) + 2.0 || !los) {
            self.out.push(Act::Stop);
        } else {
            self.release(sp, p.aim, p.friend);
        }
    }

    /// Run to the nearest friend to call it to help (or run away, if there is none): `Flee for assist`.
    fn flee(&mut self, near: &[Friend]) -> Outcome {
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
    fn begin_fight(&mut self) {
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
    fn end_fight(&mut self) {
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

    /// (Tests) Advance `dt` seconds against a plain player standing at `target`; returns the damage of the swing that landed, if one did.
    #[cfg(test)]
    fn update(&mut self, dt: f32, target: Vec3, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool) -> Option<f32> {
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
            State::Unborn => {}
            State::Dead(_) => self.tick_dead(dt),
            State::Flinch(t) => self.state = if t + dt >= FLINCH_TIME { State::Chase } else { State::Flinch(t + dt) },
            State::Idle | State::Wander(_) => {
                let noticed = aggro_range(self.def.level, watch.level) * if self.hidden { SNEAK_AGGRO * self.notice } else { 1.0 };
                if self.def.is_hostile() && !self.def.brain.calm && (target - self.pos).length() < noticed {
                    self.state = State::Chase;
                    self.sounds.push(Cry::Aggro);
                } else if self.def.attackable() || !self.route.is_empty() {
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
    fn think(&mut self, dt: f32, watch: &Watch) {
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
    fn shot_anim(&self) -> u16 {
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
    fn spell_anims(&self, spell: u32, anims: &SpellAnims) -> (u16, u16) {
        let (ready, cast) = anims.get(&spell).copied().unwrap_or_default();
        let shot = content::classic_spell(spell).is_some_and(|s| s.is_ranged_attack()) && cast.is_none();
        (ready.unwrap_or(if shot { self.shot_anim() } else { READY_SPELL }), cast.unwrap_or(if shot { self.shot_anim() } else { CAST_SPELL }))
    }

    /// The animation the creature plays; `anims` are the ones its spells ask for (the stance held while casting, the cast).
    fn animation(&self, anims: &SpellAnims) -> u16 {
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

/// What a kind of beast needs to be drawn: its model with the skins, shared by every creature of the kind.
struct BeastRig {
    m2: M2,
    template: Arc<ModelCpu>,
    /// The plain standing sequence; the other idle sequences are fidgets that come with a growl.
    idle_base: usize,
    scale: f32,
}

/// The model of a kind of beast with its skins (none for a person).
fn beast_rig(def: &CreatureDef, mount: &Mount, cache: &ModelCache) -> Result<Option<BeastRig>> {
    let Body::Beast { model, skins, scale } = &def.body else { return Ok(None) };
    let m2 = M2::parse(&mount.read(model)?).with_context(|| format!("{} model", def.name))?;
    let skins: Vec<(u32, &str)> = skins.iter().map(|(slot, file)| (*slot, file.as_str())).collect();
    let template = cache.creature(mount, model, &skins)?;
    let idle_base = m2.sequences.iter().position(|s| s.id == STAND).unwrap_or(0);
    Ok(Some(BeastRig { m2, template, idle_base, scale: *scale }))
}

/// A person on screen: a character of some race and sex dressed in his outfit, standing.
pub(crate) struct Person {
    character: Character,
    /// The body, dressed, and what hangs on it.
    worn: Worn,
    /// Whether it was swinging last frame, to start the swing once; and whether it was holding the stance of a spell being cast, or playing its release.
    swinging: bool,
    casting: bool,
    released: bool,
    /// How tall the model is, yards (a tauren is more than a gnome).
    height: f32,
    /// The animation it holds while it stands (a state emote), and whether it runs when it strolls: set by its creature before each pose.
    emote: Option<u16>,
    run: bool,
}

/// How tall a human man is, yards: what the heights below (the quest mark, the name, the click) were made for.
const HUMAN_HEIGHT: f32 = 2.0;

impl Person {
    /// The same dressing as the player's: item textures on the skin, a weapon and a shield in the hands, a helmet, shoulders.
    pub(crate) fn load(outfit: &[usize], look: &Appearance, mount: &Mount, renderer: &mut Renderer, gpu: &Gpu, key: &str) -> Result<Person> {
        let mut equipment: [Option<&'static items::ItemDef>; SLOTS] = [None; SLOTS];
        for &i in outfit {
            let def = items::get(i);
            let places: Vec<usize> = (0..SLOTS).filter(|&s| def.wear.fits(s)).collect();
            if let Some(&slot) = places.iter().find(|&&s| equipment[s].is_none()).or(places.first()) {
                equipment[slot] = Some(def);
            }
        }
        Person::dress(&equipment, look, mount, renderer, gpu, key)
    }

    /// A character of this look in what is worn.
    pub(crate) fn dress(equipment: &[Option<&'static items::ItemDef>; SLOTS], look: &Appearance, mount: &Mount, renderer: &mut Renderer, gpu: &Gpu, key: &str) -> Result<Person> {
        let cache = ModelCache::default();
        let mut character = Character::load(mount, &cache, key, look)?;
        let mut worn = Worn::new(&character, renderer, gpu);
        worn.dress(&mut character, look, mount, &cache, renderer, gpu, equipment)?;
        let height = character.model.vertices.iter().map(|v| v.pos[1]).fold(0.0, f32::max).max(0.5);
        Ok(Person { character, worn, swinging: false, casting: false, released: false, height, emote: None, run: false })
    }

    /// Stand, run after the player, swing the weapon for a bite, or fall and lie (the model's own death animation) when dead.
    #[allow(clippy::too_many_arguments)]
    fn pose(&mut self, dt: f32, pos: Vec3, yaw: f32, state: State, moving: bool, anims: (u16, u16), forward: Vec3, viewer: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        let swinging = matches!(state, State::Bite(..));
        if swinging && !self.swinging {
            self.character.attack(BITE_TIME);
        }
        self.swinging = swinging;
        // A spell: the stance is held while it is cast, and the cast is played once when it goes off.
        let casting = matches!(state, State::Cast(_, _, _, false));
        if casting != self.casting {
            self.character.set_precast(casting.then_some(anims.0));
            self.casting = casting;
        }
        let released = matches!(state, State::Cast(_, _, _, true));
        if released && !self.released {
            self.character.play_once(anims.1, CAST_AFTER + 0.3);
        }
        self.released = released;
        let speed = match state {
            State::Chase if moving => RUN_SPEED,
            State::Flee(..) | State::Kite(_) => RUN_SPEED,
            State::Return(t) if t >= RETURN_WAIT => RUN_SPEED,
            State::Wander(_) if self.run => RUN_SPEED,
            State::Wander(_) => WALK_SPEED,
            _ => 0.0,
        };
        match state {
            State::Dead(seconds) => self.character.lie(seconds * 1000.0),
            _ => self.character.update(dt, &Motion { speed, forward_speed: speed, emote: self.emote.filter(|_| state == State::Idle), ..Motion::default() }, true),
        }
        renderer.update_entity_vertices(gpu, self.worn.body, &self.character.vertices);
        let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
        self.worn.place(renderer, &self.character, pos, orient, false, false);
        self.worn.animate(dt, forward, viewer, renderer, gpu, &self.character, pos, orient, false, false);
    }

    /// Lie as a ragdoll poses the body (`bones`, none: as it lay), with what hangs on it.
    #[allow(clippy::too_many_arguments)]
    fn tumble(&mut self, bones: Option<Vec<Mat4>>, dt: f32, pos: Vec3, orient: Mat4, forward: Vec3, viewer: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        if let Some(bones) = bones {
            self.character.pose_with(bones);
            renderer.update_entity_vertices(gpu, self.worn.body, &self.character.vertices);
        }
        self.worn.place(renderer, &self.character, pos, orient, false, false);
        self.worn.animate(dt, forward, viewer, renderer, gpu, &self.character, pos, orient, false, false);
    }

    /// Standing, turned north, for the editor's picture; returns the entities to draw.
    pub(crate) fn stand(&mut self, dt: f32, pos: Vec3, forward: Vec3, renderer: &mut Renderer, gpu: &Gpu) -> Vec<usize> {
        self.pose(dt, pos, 0.0, State::Idle, false, (READY_SPELL, CAST_SPELL), forward, pos - forward * 5.0, renderer, gpu);
        self.worn.entities()
    }

    /// Standing and facing `yaw` (radians, 0 north, clockwise), for the editor's ghost; returns the entities to draw.
    pub(crate) fn stand_facing(&mut self, dt: f32, pos: Vec3, yaw: f32, renderer: &mut Renderer, gpu: &Gpu) -> Vec<usize> {
        let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        self.pose(dt, pos, yaw, State::Idle, false, (READY_SPELL, CAST_SPELL), forward, pos - forward * 5.0, renderer, gpu);
        self.worn.entities()
    }

    /// Give the entities back to the renderer.
    pub(crate) fn retire(self, renderer: &mut Renderer) {
        self.worn.retire(renderer);
    }

    /// How much taller than a human man.
    fn tall(&self) -> f32 {
        self.height / HUMAN_HEIGHT
    }

    pub(crate) fn hide(&self, pos: Vec3, renderer: &mut Renderer) {
        self.worn.hide(renderer, pos);
    }
}

enum Visual {
    Beast {
        entity: usize,
        anim: Channel,
        /// The idle sequence last seen, to notice when it changes to a fidget.
        last_seq: usize,
        /// The kind whose model it shows (a waypoint script can change it).
        shown: usize,
    },
    Person(Person),
    /// Not made yet (it is near, but its turn has not come) or no longer there.
    None,
}

/// A quest mark's model, shared by every creature that shows it.
struct MarkerRig {
    m2: M2,
    template: Arc<ModelCpu>,
}

struct Creature {
    mob: Mob,
    /// The creature whose script summons this one (it waits in the pool of summoned creatures until then).
    parent: Option<usize>,
    /// The place it stands at, while it is in the world (none for a summoned one, or for a slot that is free).
    site: Option<usize>,
    visual: Visual,
    /// The entities of its three quest marks (those creatures that can be talked to have them), and the one shown.
    marks: Option<[usize; 3]>,
    want: Option<Marker>,
    /// It fell as a ragdoll when it died, and lies as that until it is back; whether it was dead at the last frame.
    ragdoll: Option<Ragdoll>,
    was_dead: bool,
    /// Seconds not posed yet and the frame it is at of the few it is posed every (far creatures are posed less often).
    owed: f32,
    beat: u8,
}

/// The rectangle (x0, y0, x1, y1 in pixels) that a thing `height` yards tall standing at `feet` covers on a `size` screen seen
/// through `camera`; `width` is half its width as a share of its height on the screen.
pub fn screen_box(camera: &Camera, size: (u32, u32), feet: Vec3, height: f32, width: f32) -> Option<[f32; 4]> {
    let (fx, fy) = Creatures::on_screen(camera, size, feet)?;
    let (_, hy) = Creatures::on_screen(camera, size, feet + Vec3::Y * height)?;
    let half = ((fy - hy).abs() * width).max(8.0);
    Some([fx - half, hy.min(fy) - 4.0, fx + half, hy.max(fy) + 4.0])
}

/// What the target frame shows about the target.
pub struct TargetView {
    /// Which creature it is (for noticing a change of target).
    pub id: usize,
    pub name: String,
    pub level: u32,
    /// Health as a share of the most it has.
    pub fraction: f32,
    pub dead: bool,
    pub hostile: bool,
    pub neutral: bool,
}

/// How far the target is let go at (yards), and how far the next one is looked for.
const TARGET_LOSE: f32 = 60.0;
const TARGET_RANGE: f32 = 40.0;

/// A name floating over a creature: where on the screen (pixels, the middle of the text) and in which colour.
pub struct Label {
    pub text: String,
    pub color: [f32; 4],
    pub x: f32,
    pub y: f32,
}

/// A place of the map where a creature of some kind stands (a spawn of the content): while the player is far it is only this.
struct Site {
    kind: usize,
    pos: Vec3,
    yaw: f32,
    wander: f32,
    respawn: Option<f32>,
    route: Vec<Stop>,
    /// Its place among the creatures while it is in the world.
    slot: Option<usize>,
}

fn cell_of(at: Vec3) -> (i32, i32) {
    ((at.x / CELL).floor() as i32, (at.z / CELL).floor() as i32)
}

pub struct Creatures {
    /// By kind (index into the content's creatures): the model of every kind of beast in the world.
    rigs: Vec<Option<BeastRig>>,
    /// The creatures in the world now (those near the player); a slot that is free holds an unborn creature and is in `free`.
    creatures: Vec<Creature>,
    sites: Vec<Site>,
    /// The sites by cell and by kind.
    grid: HashMap<(i32, i32), Vec<usize>>,
    by_kind: HashMap<usize, Vec<usize>>,
    free: Vec<usize>,
    /// What a save says of a creature whose site is not in the world now, by site.
    pending: HashMap<usize, MobSave>,
    mount: Arc<Mount>,
    cache: ModelCache,
    budget: usize,
    scan: f32,
    markers: Vec<MarkerRig>,
    viewer: Vec3,
    rng: Rng,
    clock_ms: f32,
    vertices: Vec<ModelVertex>,
    /// Kinds of the creatures killed since `take_deaths`.
    deaths: Vec<usize>,
    /// The creature the player has selected.
    target: Option<usize>,
    /// The quest items that the active quests want (`QuestLog::wanted`): what dies now may drop them, and no other quest item.
    wanted: Vec<usize>,
    /// Whether the creatures that die fall as ragdolls (the interface options).
    pub ragdolls: bool,
    /// The animations the creatures' spells ask for.
    anims: SpellAnims,
}

/// Most creatures of one kind that a creature has waiting to be summoned.
const MOST_SUMMONED: usize = 6;

impl Creatures {
    /// The creatures at `spawns`: (kind, feet position in render space, heading in radians, yards it strolls from there). They are made as the player comes near.
    pub fn load(mount: &Arc<Mount>, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, spawns: &[(usize, Vec3, f32, f32)]) -> Result<Creatures> {
        let defs = &content::get().creatures;
        let mut rigs: Vec<Option<BeastRig>> = defs.iter().map(|_| None).collect();
        let rng = Rng(0x5EED_1234);
        // The models of the marks; without them (a missing file) the creatures simply show none.
        let markers: Vec<MarkerRig> = MARKER_MODELS
            .iter()
            .map_while(|path| {
                let m2 = M2::parse(&mount.read(path).ok()?).ok()?;
                Some(MarkerRig { m2, template: cache.plain(mount, path).ok()? })
            })
            .collect();
        if markers.len() < MARKER_MODELS.len() {
            log::warn!("the quest mark models are missing");
        }
        let markers = if markers.len() == MARKER_MODELS.len() { markers } else { Vec::new() };
        // The looks that the waypoint scripts turn creatures into.
        for id in content::get().spawns.iter().flat_map(|s| &s.path).flat_map(|w| &w.steps).filter_map(|(_, step)| if let content::Step::Morph(id) = step { Some(id) } else { None }) {
            if let Some(kind) = defs.iter().position(|d| d.id == *id).filter(|&k| rigs[k].is_none()) {
                rigs[kind] = beast_rig(&defs[kind], mount, cache)?;
            }
        }
        let sites: Vec<Site> = spawns.iter().map(|&(kind, pos, yaw, wander)| Site { kind, pos, yaw, wander, respawn: None, route: Vec::new(), slot: None }).collect();
        let (mut grid, mut by_kind): (HashMap<(i32, i32), Vec<usize>>, HashMap<usize, Vec<usize>>) = Default::default();
        for (i, site) in sites.iter().enumerate() {
            grid.entry(cell_of(site.pos)).or_default().push(i);
            by_kind.entry(site.kind).or_default().push(i);
        }
        let _ = renderer;
        let _ = gpu;
        Ok(Creatures { rigs, creatures: Vec::new(), sites, grid, by_kind, free: Vec::new(), pending: HashMap::new(), mount: mount.clone(), cache: ModelCache::default(), budget: MAKE_PER_FRAME, scan: SCAN, markers, viewer: Vec3::ZERO, rng, clock_ms: 0.0, vertices: Vec::new(), deaths: Vec::new(), target: None, wanted: Vec::new(), ragdolls: true, anims: SpellAnims::new() })
    }

    /// The creatures at their places that have a model: (kind id, north and west of the place, where it is now in render space).
    pub fn standing(&self) -> Vec<(String, f32, f32, Vec3)> {
        self.creatures
            .iter()
            .filter(|c| c.site.is_some() && !matches!(c.visual, Visual::None) && !c.mob.is_dead())
            .map(|c| (c.mob.def.id.clone(), -c.mob.home.z, -c.mob.home.x, c.mob.pos))
            .collect()
    }

    /// How many creatures are in the world now, and how many of them have their model made (the log of a picture taken from the command line says it).
    pub fn counts(&self) -> (usize, usize) {
        let live = self.creatures.iter().filter(|c| c.site.is_some());
        (live.clone().count(), live.filter(|c| !matches!(c.visual, Visual::None)).count())
    }

    /// How many creatures get their model in one frame at most (the first frames of a picture taken from the command line want them all).
    pub fn set_budget(&mut self, most: usize) {
        self.budget = most;
    }

    /// Put a creature into a free slot, or a new one.
    fn alloc(&mut self, creature: Creature) -> usize {
        match self.free.pop() {
            Some(i) => {
                self.creatures[i] = creature;
                i
            }
            None => {
                self.creatures.push(creature);
                self.creatures.len() - 1
            }
        }
    }

    /// The place at `site` becomes a creature in the world (without a model yet), with the creatures that its script summons waiting behind it.
    fn activate(&mut self, site: usize) {
        let defs = &content::get().creatures;
        let s = &self.sites[site];
        let (kind, pos, yaw, wander) = (s.kind, s.pos, s.yaw, s.wander);
        let mut mob = Mob::new(kind, pos, yaw);
        mob.wander = wander;
        mob.respawn = s.respawn.unwrap_or(mob.def.respawn);
        mob.route = s.route.clone();
        if let Some(saved) = self.pending.remove(&site) {
            mob.restore(&saved);
        }
        let slot = self.alloc(Creature { mob, parent: None, site: Some(site), visual: Visual::None, marks: None, want: None, ragdoll: None, was_dead: false, owed: 0.0, beat: 0 });
        self.sites[site].slot = Some(slot);
        // Behind it wait the creatures that its script summons (twice as many as there are summon actions, so that a summon that repeats finds some).
        let mut waiting: Vec<(usize, usize)> = Vec::new();
        for act in defs[kind].brain.events.iter().flat_map(|e| &e.acts).filter(|a| a.0 == brain::act::SUMMON) {
            let id = format!("c{}", act.1);
            let Some(summoned) = content::get().creature_numbers.get(&id).copied() else { continue };
            match waiting.iter_mut().find(|w| w.0 == summoned) {
                Some(w) => w.1 = (w.1 + 2).min(MOST_SUMMONED),
                None => waiting.push((summoned, 2)),
            }
        }
        for (summoned, count) in waiting {
            for _ in 0..count {
                let mut mob = Mob::new(summoned, pos, yaw);
                (mob.wander, mob.summoned, mob.state) = (wander, true, State::Unborn);
                self.alloc(Creature { mob, parent: Some(slot), site: None, visual: Visual::None, marks: None, want: None, ragdoll: None, was_dead: false, owed: 0.0, beat: 0 });
            }
        }
    }

    /// Take creature `i` and what it summons out of the world (its models go back), leaving free slots.
    fn deactivate(&mut self, i: usize, renderer: &mut Renderer) {
        let site = self.creatures[i].site;
        let kids: Vec<usize> = (0..self.creatures.len()).filter(|&j| self.creatures[j].parent == Some(i)).collect();
        for j in kids.into_iter().chain([i]) {
            let c = &mut self.creatures[j];
            match std::mem::replace(&mut c.visual, Visual::None) {
                Visual::Beast { entity, .. } => renderer.remove_entity(entity),
                Visual::Person(p) => p.retire(renderer),
                Visual::None => {}
            }
            c.marks.take().into_iter().flatten().for_each(|e| renderer.remove_entity(e));
            (c.parent, c.site, c.want, c.ragdoll, c.was_dead) = (None, None, None, None, false);
            c.mob.state = State::Unborn;
            self.free.push(j);
        }
        if let Some(site) = site {
            self.sites[site].slot = None;
        }
        if self.target == Some(i) {
            self.target = None;
        }
    }

    /// Look at who is near: the places within `ACTIVATE` yards become creatures, and the ones at rest beyond `DEACTIVATE` are put away.
    fn stream(&mut self, player: Vec3, renderer: &mut Renderer) {
        let (cx, cz) = cell_of(player);
        let reach = (ACTIVATE / CELL).ceil() as i32;
        let mut wake = Vec::new();
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                for &s in self.grid.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if self.sites[s].slot.is_none() && flat(self.sites[s].pos - player).length() <= ACTIVATE {
                        wake.push(s);
                    }
                }
            }
        }
        for s in wake {
            self.activate(s);
        }
        // Only a creature that is at rest goes: not one that fights, runs home, lies dead (its time to come back is counted), or is the target.
        let gone: Vec<usize> = (0..self.creatures.len())
            .filter(|&i| {
                let c = &self.creatures[i];
                c.site.is_some()
                    && self.target != Some(i)
                    && matches!(c.mob.state, State::Idle | State::Wander(_))
                    && flat(c.mob.pos - player).length() > DEACTIVATE
                    && flat(c.mob.home - player).length() > DEACTIVATE
            })
            .collect();
        for i in gone {
            self.deactivate(i, renderer);
        }
    }

    /// Give the models to the creatures that are near and have none, the nearest first, a few in a frame (a model is read and dressed for each).
    fn make_visible(&mut self, player: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        let mut need: Vec<(f32, usize)> = (0..self.creatures.len())
            .filter(|&i| matches!(self.creatures[i].visual, Visual::None) && self.creatures[i].mob.state != State::Unborn)
            .map(|i| (flat(self.creatures[i].mob.pos - player).length(), i))
            .filter(|&(d, _)| d <= DRAW_RANGE)
            .collect();
        need.sort_by(|a, b| a.0.total_cmp(&b.0));
        let started = Instant::now();
        for (n, (_, i)) in need.into_iter().enumerate() {
            if n >= self.budget || (n > 0 && started.elapsed() > MAKE_TIME) {
                break;
            }
            if let Err(e) = self.make(i, renderer, gpu) {
                // It is not in the world then (and is not tried again).
                log::warn!("{}: {e:#}", self.creatures[i].mob.def.name);
                self.creatures[i].mob.state = State::Unborn;
            }
        }
    }

    /// The model of creature `i` (and its quest marks).
    fn make(&mut self, i: usize, renderer: &mut Renderer, gpu: &Gpu) -> Result<()> {
        let (kind, def) = (self.creatures[i].mob.kind, self.creatures[i].mob.def);
        // People of one kind share the picture of their skin (they are dressed alike), under a key of the kind.
        let key = format!("npc/{kind}");
        let visual = match &def.body {
            Body::Beast { .. } => {
                if self.rigs[kind].is_none() {
                    self.rigs[kind] = beast_rig(def, &self.mount, &self.cache)?;
                }
                let rig = self.rigs[kind].as_ref().unwrap();
                Visual::Beast { entity: renderer.add_room_lit_entity(gpu, &rig.template), anim: Channel::new(&rig.m2, STAND, &mut self.rng), last_seq: usize::MAX, shown: kind }
            }
            // A look that does not exist for the race (a face, a hair style) is shown as the default of the race instead.
            Body::Humanoid { outfit, look } => Visual::Person(match Person::load(outfit, look, &self.mount, renderer, gpu, &key) {
                Ok(person) => person,
                Err(e) => {
                    log::warn!("{}: {e:#}; showing the default look", def.name);
                    Person::load(outfit, &Appearance { race: look.race, sex: look.sex, skin: 0, face: 0, hair_style: 0, hair_color: 0, beard: 0, baked: 0 }, &self.mount, renderer, gpu, &key)?
                }
            }),
        };
        // Whoever can be talked to may have a quest mark over its head: one entity for each of the three marks.
        let marks = (def.gossip.is_some() && !self.markers.is_empty()).then(|| [0, 1, 2].map(|m| renderer.add_entity(gpu, &self.markers[m].template)));
        let c = &mut self.creatures[i];
        (c.visual, c.marks) = (visual, marks);
        Ok(())
    }

    /// Where the creatures of the given kinds are now (render space): the living ones, else the corpses (which wait to come back); of those
    /// that are not in the world now, where they stand. The nearest few to the viewer.
    pub fn positions(&self, kinds: &[usize]) -> Vec<Vec3> {
        let of_kind = || self.creatures.iter().filter(|c| c.site.is_some() || c.parent.is_some()).map(|c| &c.mob).filter(|m| kinds.contains(&m.kind));
        let mut all: Vec<Vec3> = of_kind().filter(|m| !m.is_dead()).map(|m| m.pos).collect();
        all.extend(kinds.iter().filter_map(|k| self.by_kind.get(k)).flatten().map(|&s| &self.sites[s]).filter(|s| s.slot.is_none()).map(|s| s.pos));
        if all.is_empty() {
            all = of_kind().map(|m| m.pos).collect();
        }
        all.sort_by(|a, b| flat(*a - self.viewer).length().total_cmp(&flat(*b - self.viewer).length()));
        all.truncate(MOST_LEADS);
        all
    }

    /// The creatures worth keeping in a save (the ones hurt, dead, fighting or carrying loot; the rest start afresh), and the ones kept for places not in the world now.
    pub fn snapshot(&self) -> Vec<MobSave> {
        self.creatures.iter().filter(|c| c.site.is_some() && c.mob.worth_saving()).map(|c| c.mob.snapshot()).chain(self.pending.values().cloned()).collect()
    }

    /// Give the sites their own respawn times and patrols (the n-th is the n-th spawned): the seconds to come back, and the places to walk to.
    pub fn set_routes(&mut self, routes: Vec<(Option<f32>, Vec<content::Waypoint>)>) {
        for (site, (respawn, path)) in self.sites.iter_mut().zip(routes) {
            site.respawn = respawn;
            site.route = path
                .into_iter()
                .map(|w| Stop { at: crate::coords::wow_to_render([w.at.0, w.at.1, 0.0]), wait: w.wait, facing: w.facing.map(f32::to_radians), steps: w.steps })
                .collect();
        }
    }

    /// The site a saved creature stood at (the same kind with the same home: the content may have changed since).
    fn site_of(&self, saved: &MobSave) -> Option<usize> {
        let (cx, cz) = cell_of(saved.home);
        let defs = &content::get().creatures;
        (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dz| (cx + dx, cz + dz)))
            .flat_map(|cell| self.grid.get(&cell).into_iter().flatten().copied())
            .find(|&s| defs[self.sites[s].kind].id == saved.kind && flat(self.sites[s].pos - saved.home).length() <= 1.0)
    }

    /// Put saved creatures back; those that do not match a place of the world any more start afresh. A creature whose place is not in the world now comes back when it is.
    pub fn restore(&mut self, saved: &[MobSave]) {
        let mut back = 0;
        for s in saved {
            let Some(site) = self.site_of(s) else { continue };
            match self.sites[site].slot {
                Some(slot) => back += self.creatures[slot].mob.restore(s) as usize,
                None => {
                    self.pending.insert(site, s.clone());
                    back += 1;
                }
            }
        }
        log::info!("{back} of {} creatures back as saved", saved.len());
    }

    /// Where an attachment of creature `who` (1 the right hand, 2 the left) is in the world, if it is a person: what the glows of its spells follow.
    pub fn hand(&self, who: usize, id: u32) -> Option<Mat4> {
        let c = self.creatures.get(who)?;
        let Visual::Person(p) = &c.visual else { return None };
        let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw);
        Some(Mat4::from_translation(c.mob.pos) * orient * crate::character::to_render(p.character.attachment(id)?))
    }

    /// The animations the spells of creatures ask for (the stance held while casting, the cast).
    pub fn set_spell_anims(&mut self, anims: SpellAnims) {
        self.anims = anims;
    }

    /// How tall creature `c` is, yards (where its spells are shown).
    fn height_of(c: &Creature) -> f32 {
        match (&c.visual, &c.mob.def.body) {
            (Visual::Person(p), _) => HUMAN_HEIGHT * p.tall(),
            (_, Body::Beast { scale, .. }) => scale.max(0.8),
            _ => 1.5,
        }
    }

    /// Advance every creature and pose it. Returns what they did that the game acts on (the swings that landed on the player, the spells begun, stopped
    /// and gone off), and the sounds the creatures made with the place they came from.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, watch: &Watch, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool, mut physics: Option<&mut Bodies>, renderer: &mut Renderer, gpu: &Gpu) -> (Vec<Foe>, Vec<(Sfx, Vec3)>) {
        self.clock_ms += dt * 1000.0;
        let player = watch.feet;
        self.scan += dt;
        if self.scan >= SCAN {
            self.scan = 0.0;
            self.stream(player, renderer);
        }
        self.make_visible(player, renderer, gpu);
        let (mut foes, mut cries) = (Vec::new(), Vec::new());
        // Who is about, for the creatures that look after friends, call them and run to them.
        let roster: Vec<Friend> = self
            .creatures
            .iter()
            .enumerate()
            .filter(|(_, c)| c.mob.def.attackable() && !c.mob.is_dead() && flat(c.mob.pos - player).length() <= DRAW_RANGE)
            .map(|(index, c)| Friend { index, pos: c.mob.pos, faction: c.mob.def.brain.faction, dist: 0.0, health: c.mob.health, max: c.mob.def.health, auras: c.mob.aura_ids() })
            .collect();
        let (mut casts, mut summons, mut assists) = (Vec::new(), Vec::new(), Vec::new());
        for (i, c) in self.creatures.iter_mut().enumerate() {
            let mouth = c.mob.pos + Vec3::Y * 0.8;
            let kind = c.mob.kind;
            cries.extend(c.mob.sounds.drain(..).map(|cry| (Sfx::Cry(cry, kind), mouth)));
            if c.mob.state == State::Unborn || flat(c.mob.pos - player).length() > DRAW_RANGE {
                // Out of reach nothing is updated: one that was after the player (as the save left it) gives up, or the player would stay in combat.
                if c.mob.state.fights() {
                    c.mob.state = State::Return(0.0);
                }
                c.mob.tick_dead(dt);
                match &c.visual {
                    Visual::Beast { entity, .. } => renderer.set_entity(*entity, c.mob.pos, Mat4::IDENTITY, false),
                    Visual::Person(p) => p.hide(c.mob.pos, renderer),
                    Visual::None => {}
                }
                for e in c.marks.into_iter().flatten() {
                    renderer.set_entity(e, c.mob.pos, Mat4::IDENTITY, false);
                }
                continue;
            }
            // Near, but its model is yet to be made.
            if matches!(c.visual, Visual::None) {
                continue;
            }
            // The quest mark: a small model that spins above the head (its own animation), only the one that is wanted.
            if let Some(marks) = c.marks {
                for (m, &entity) in marks.iter().enumerate() {
                    let shown = c.want.map(|w| w as usize) == Some(m);
                    // The model's front is turned to the viewer (the same heading arithmetic as for a creature that faces him).
                    let tall = if let Visual::Person(p) = &c.visual { p.tall() } else { 1.0 };
                    let at = c.mob.pos + Vec3::Y * MARKER_HEIGHT * tall;
                    let to = flat(self.viewer - at);
                    let facing = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - to.x.atan2(-to.z));
                    renderer.set_entity(entity, at, facing, shown);
                    if shown {
                        let rig = &self.markers[m];
                        let seq = rig.m2.sequences.iter().position(|q| q.id == STAND).unwrap_or(0);
                        let (start, length) = (rig.m2.sequences[seq].start as f32, (rig.m2.sequences[seq].end - rig.m2.sequences[seq].start).max(1) as f32);
                        let bones = anim::bone_matrices(&rig.m2, seq, start + self.clock_ms % length, self.clock_ms);
                        anim::skin(&rig.m2, &bones, &rig.template.vertices, &mut self.vertices);
                        renderer.update_entity_vertices(gpu, entity, &self.vertices);
                    }
                }
            }
            (c.mob.hidden, c.mob.notice) = (watch.sneaking.is_some(), watch.sneaking.unwrap_or(1.0));
            let height = Self::height_of(c);
            let acts = c.mob.step(dt, watch, i, &roster, &ground, &blocked);
            // What the damage over time slew is a kill of the player's.
            if std::mem::take(&mut c.mob.slain_over) {
                self.deaths.push(kind);
                c.mob.loot = c.mob.loot.take().map(|mut l| { l.only_wanted(&self.wanted); l }).filter(|l| !l.is_empty());
            }
            for act in acts {
                match act {
                    Act::Melee(damage) => foes.push(Foe::Melee(damage)),
                    Act::Begin(spell) => foes.push(Foe::Begin { who: i, spell, at: c.mob.pos, yaw: c.mob.yaw, height }),
                    Act::Stop => foes.push(Foe::Stop { who: i }),
                    Act::Cast(cast) => casts.push((i, cast, height)),
                    Act::Summon { entry, ms } => summons.push((i, entry, ms)),
                    Act::Assist(radius) => assists.push((i, radius)),
                }
            }
            // A creature that dies falls as a ragdoll if the physics has room for it, until it is back.
            let dead = matches!(c.mob.state, State::Dead(_));
            if let Some(r) = c.ragdoll.take() {
                match physics.as_deref_mut() {
                    Some(p) if dead && r.alive(p) => c.ragdoll = Some(r),
                    Some(p) => r.remove(p),
                    None => {}
                }
            }
            let fresh = dead && !c.was_dead && matches!(c.mob.state, State::Dead(t) if t < 0.5);
            c.was_dead = dead;
            let push = flat(c.mob.pos - player).normalize_or_zero() * RAGDOLL_PUSH + Vec3::Y * 1.5;
            let spawn_here = fresh && self.ragdolls && flat(c.mob.pos - player).length() < RAGDOLL_REACH;
            // Far from the player a creature is posed less often (its model is skinned and sent to the graphics card each time): every second frame
            // beyond 45 yards, every fourth beyond 90. One that fights, lies dead or falls is posed at once.
            let away = flat(c.mob.pos - player).length();
            c.owed += dt;
            c.beat = (c.beat + 1) % if away < POSE_NEAR { 1 } else if away < POSE_FAR { 2 } else { 4 };
            if c.beat != 0 && c.ragdoll.is_none() && !dead && !c.mob.state.fights() && !spawn_here {
                continue;
            }
            let dt = std::mem::take(&mut c.owed);
            match &mut c.visual {
                Visual::Beast { entity, anim, last_seq, shown } => {
                    // A script changed how it looks: the model of the new kind takes the place of the old.
                    if *shown != c.mob.look {
                        match self.rigs[c.mob.look].as_ref() {
                            Some(rig) => {
                                renderer.remove_entity(*entity);
                                (*entity, *anim, *last_seq, *shown) = (renderer.add_room_lit_entity(gpu, &rig.template), Channel::new(&rig.m2, STAND, &mut self.rng), usize::MAX, c.mob.look);
                            }
                            None => c.mob.look = *shown,
                        }
                    }
                    let rig = self.rigs[*shown].as_ref().expect("a beast has its rig");
                    let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw) * Mat4::from_scale(Vec3::splat(rig.scale));
                    if let (true, Some(p)) = (spawn_here, physics.as_deref_mut()) {
                        let bones = anim::bone_matrices_with(&rig.m2, anim.mix(&rig.m2), self.clock_ms, &[], &[]);
                        c.ragdoll = Ragdoll::spawn(p, &rig.m2, &rig.template, bones, Mat4::from_translation(c.mob.pos) * orient, push);
                    }
                    if let Some(rd) = c.ragdoll.as_mut() {
                        if let Some(bones) = physics.as_deref_mut().and_then(|p| rd.bones(p)) {
                            anim::skin(&rig.m2, &bones, &rig.template.vertices, &mut self.vertices);
                            renderer.update_entity_vertices(gpu, *entity, &self.vertices);
                            // The corpse is where it has been moved to (what is looted and clicked on).
                            if let Some(at) = physics.as_deref().and_then(|p| rd.corpse_at(p)) {
                                c.mob.pos = Vec3::new(at.x, ground(at.x, at.z, at.y).unwrap_or(c.mob.pos.y), at.z);
                            }
                        }
                        renderer.set_entity(*entity, rd.origin(), orient, true);
                        continue;
                    }
                    anim.request(&rig.m2, c.mob.animation(&self.anims), true, &mut self.rng);
                    anim.advance(&rig.m2, dt * 1000.0, 1.0, true, &mut self.rng);
                    // An idle that turns into a fidget (a look around, a shake) comes with a growl.
                    if c.mob.state == State::Idle && anim.seq() != *last_seq && anim.seq() != rig.idle_base && *last_seq != usize::MAX {
                        cries.push((Sfx::Cry(Cry::Fidget, kind), mouth));
                    }
                    *last_seq = anim.seq();
                    let bones = anim::bone_matrices_with(&rig.m2, anim.mix(&rig.m2), self.clock_ms, &[], &[]);
                    anim::skin(&rig.m2, &bones, &rig.template.vertices, &mut self.vertices);
                    renderer.update_entity_vertices(gpu, *entity, &self.vertices);
                    // Batches that the model shows only in some animations (the logs a peasant carries) fade in and out by its tracks.
                    if let Some(tracks) = &rig.template.tracks {
                        let pose = anim.mix(&rig.m2).to;
                        let looks: Vec<_> = rig.template.batches.iter().map(|b| tracks.look_in(b.refs, pose.seq, pose.time, self.clock_ms)).collect();
                        renderer.set_entity_looks(gpu, *entity, &looks);
                    }
                    let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw) * Mat4::from_scale(Vec3::splat(rig.scale));
                    renderer.set_entity(*entity, c.mob.pos, orient, true);
                }
                Visual::Person(person) => {
                    let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw);
                    if let (true, Some(p)) = (spawn_here, physics.as_deref_mut()) {
                        c.ragdoll = Ragdoll::spawn(p, &person.character.m2, &person.character.model, person.character.bones.clone(), Mat4::from_translation(c.mob.pos) * orient, push);
                    }
                    if let Some(rd) = c.ragdoll.as_mut() {
                        let bones = physics.as_deref_mut().and_then(|p| rd.bones(p));
                        if bones.is_some() {
                            if let Some(at) = physics.as_deref().and_then(|p| rd.corpse_at(p)) {
                                c.mob.pos = Vec3::new(at.x, ground(at.x, at.z, at.y).unwrap_or(c.mob.pos.y), at.z);
                            }
                        }
                        person.tumble(bones, dt, rd.origin(), orient, (c.mob.pos + Vec3::Y - self.viewer).normalize_or_zero(), self.viewer, renderer, gpu);
                        continue;
                    }
                    let anims = if let State::Cast(spell, ..) = c.mob.state { c.mob.spell_anims(spell, &self.anims) } else { (READY_SPELL, CAST_SPELL) };
                    (person.emote, person.run) = (c.mob.emote, c.mob.run);
                    person.pose(dt, c.mob.pos, c.mob.yaw, c.mob.state, c.mob.moving, anims, (c.mob.pos + Vec3::Y - self.viewer).normalize_or_zero(), self.viewer, renderer, gpu)
                }
                Visual::None => {}
            }
        }
        // Summoned creatures come out of the pool, round whoever summoned them.
        for (parent, entry, ms) in summons {
            let id = format!("c{entry}");
            let (at, yaw) = (self.creatures[parent].mob.pos, self.creatures[parent].mob.yaw);
            if let Some(j) = (0..self.creatures.len()).find(|&j| self.creatures[j].parent == Some(parent) && self.creatures[j].mob.state == State::Unborn && self.creatures[j].mob.def.id == id) {
                let (angle, radius) = (self.rng.next() as f32 / u32::MAX as f32 * std::f32::consts::TAU, 2.0 + 2.0 * (self.rng.next() as f32 / u32::MAX as f32));
                self.creatures[j].mob.summon_at(at + Vec3::new(angle.sin(), 0.0, angle.cos()) * radius, yaw, ms);
            }
        }
        // A call for help: the creatures of the faction round the caller that are not fighting turn on the player.
        for (caller, radius) in assists {
            let (at, faction) = (self.creatures[caller].mob.pos, self.creatures[caller].mob.def.brain.faction);
            for (j, c) in self.creatures.iter_mut().enumerate() {
                if j != caller && c.mob.def.brain.faction == faction && flat(c.mob.pos - at).length() <= radius {
                    c.mob.assist();
                }
            }
        }
        // Spells for friends do their work on the friends they reach.
        for (who, cast, height) in casts {
            let Some(sp) = content::classic_spell(cast.spell) else { continue };
            let faction = self.creatures[who].mob.def.brain.faction;
            let mut allies = Vec::new();
            match cast.aim {
                Aim::Itself => allies.push(self.creatures[who].mob.pos),
                Aim::Friend => {
                    let f = cast.friend.filter(|&f| f < self.creatures.len() && !self.creatures[f].mob.is_dead()).unwrap_or(who);
                    self.creatures[f].mob.apply(sp);
                    allies.push(self.creatures[f].mob.pos);
                }
                Aim::Around => {
                    let radius = sp.effects.iter().filter(|e| e.friendly()).map(|e| e.radius).fold(0.0, f32::max);
                    let at = self.creatures[who].mob.pos;
                    for (j, c) in self.creatures.iter_mut().enumerate() {
                        if j != who && !c.mob.is_dead() && c.mob.def.brain.faction == faction && flat(c.mob.pos - at).length() <= radius {
                            c.mob.apply(sp);
                            allies.push(c.mob.pos);
                        }
                    }
                    allies.push(at);
                }
                Aim::Victim => {}
            }
            foes.push(Foe::Cast { who, cast, height, allies });
        }
        (foes, cries)
    }

    /// The player's sword swing: every hostile creature it reaches (see `Mob::struck_by`) takes `damage`. Returns where
    /// above each the damage should show.
    /// `against(level, kind)` is what the blow does to a creature of that level and type: `None` when it misses (it is listed with no damage), else the damage
    /// the gear adds against its type. Each hit is listed with the creature (its index) it landed on.
    pub fn strike(&mut self, from: Vec3, yaw: f32, damage: f32, unaware_bonus: f32, against: &mut dyn FnMut(u32, u32) -> Option<f32>) -> Vec<(Vec3, f32, bool, usize)> {
        let (mut hits, mut ids) = (Vec::new(), Vec::new());
        for (i, c) in self.creatures.iter_mut().enumerate() {
            // A creature that has not noticed the player takes the blow harder (`unaware_bonus`, 1 for none).
            let sneak = matches!(c.mob.state, State::Idle | State::Wander(_)) && unaware_bonus > 1.0;
            let extra = if c.mob.in_swing(from, yaw, SWORD_REACH) { against(c.mob.def.level, c.mob.def.kind) } else { Some(0.0) };
            let dealt = (damage + extra.unwrap_or(0.0)) * if sneak { unaware_bonus } else { 1.0 };
            if extra.is_none() {
                hits.push((c.mob.pos + Vec3::Y * 0.9, 0.0, false, i));
            } else if c.mob.struck_by(from, yaw, SWORD_REACH, dealt) {
                hits.push((c.mob.pos + Vec3::Y * 0.9, dealt, sneak, i));
                ids.push(i);
                if c.mob.is_dead() {
                    self.deaths.push(c.mob.kind);
                    c.mob.loot = c.mob.loot.take().map(|mut l| { l.only_wanted(&self.wanted); l }).filter(|l| !l.is_empty());
                }
            }
        }
        // Whoever is hit becomes the target, unless a creature that is still alive is selected already.
        if self.target.is_none_or(|t| self.creatures.get(t).is_none_or(|c| c.mob.is_dead())) {
            self.target = ids.first().copied().or(self.target);
        }
        hits
    }

    /// The first living hostile creature that the flight of a missile from `a` to `b` passes through (its body is a cylinder as tall as it is and as wide as a
    /// third of that, at least 0.4 yd): it and where the missile enters it.
    pub fn arrow_hit(&self, a: Vec3, b: Vec3) -> Option<(usize, Vec3)> {
        let d = b - a;
        let hit = |i: usize| {
            let (c, pos) = (&self.creatures[i], self.creatures[i].mob.pos);
            let (h, r) = (Self::height_of(c), (Self::height_of(c) * 0.33).clamp(0.4, 2.0));
            let (f, dd) = ((a.x - pos.x, a.z - pos.z), (d.x, d.z));
            let (qa, qb, qc) = (dd.0 * dd.0 + dd.1 * dd.1, 2.0 * (f.0 * dd.0 + f.1 * dd.1), f.0 * f.0 + f.1 * f.1 - r * r);
            // Where the segment is inside the circle on the ground, as a share of its length; straight up or down it is there all along or never.
            let (t0, t1) = if qa < 1e-9 {
                if qc > 0.0 { return None } else { (0.0, 1.0) }
            } else {
                let disc = qb * qb - 4.0 * qa * qc;
                if disc < 0.0 { return None }
                ((-qb - disc.sqrt()) / (2.0 * qa), (-qb + disc.sqrt()) / (2.0 * qa))
            };
            let (t0, t1) = (t0.max(0.0), t1.min(1.0));
            // And at that height: the share of the way where it is between the feet and the head.
            let (lo, hi) = (pos.y - 0.1, pos.y + h);
            let (ya, yb) = (a.y + d.y * t0, a.y + d.y * t1);
            (t0 <= t1 && ya.max(yb) >= lo && ya.min(yb) <= hi).then(|| (t0, a + d * t0))
        };
        (0..self.creatures.len()).filter(|&i| self.enemy_at(i).is_some_and(|p| flat(p - self.viewer).length() <= DRAW_RANGE)).filter_map(|i| hit(i).map(|(t, at)| (t, i, at))).min_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, i, at)| (i, at))
    }

    pub fn target(&self) -> Option<usize> {
        self.target
    }

    /// Where creature `i` stands, if it is a hostile one that is alive (what a spell can be cast on).
    pub fn enemy_at(&self, i: usize) -> Option<Vec3> {
        let m = &self.creatures.get(i)?.mob;
        (m.def.attackable() && !m.is_dead()).then_some(m.pos)
    }

    /// A spell hurts creature `i` for `damage`. Returns where above it the damage shows. It becomes the target.
    pub fn spell_hit(&mut self, i: usize, damage: f32) -> Option<Vec3> {
        self.enemy_at(i)?;
        let c = &mut self.creatures[i];
        c.mob.hurt(damage);
        if c.mob.is_dead() {
            self.deaths.push(c.mob.kind);
            c.mob.loot = c.mob.loot.take().map(|mut l| { l.only_wanted(&self.wanted); l }).filter(|l| !l.is_empty());
        }
        self.target = Some(i);
        Some(c.mob.pos + Vec3::Y * 0.9)
    }

    /// Put something on every hostile creature within `radius` yards of `centre`.
    pub fn afflict_area(&mut self, centre: Vec3, radius: f32, status: Status) {
        for i in 0..self.creatures.len() {
            if self.enemy_at(i).is_some_and(|p| flat(p - centre).length() <= radius) {
                self.afflict(i, status);
            }
        }
    }

    /// Put something on creature `i`.
    pub fn afflict(&mut self, i: usize, status: Status) {
        if let Some(c) = self.creatures.get_mut(i) {
            c.mob.afflict(status);
        }
    }

    /// The creature that is fighting the player and is nearest to `feet` (what hits back at him is aimed at it), within a short reach.
    pub fn attacker(&self, feet: Vec3) -> Option<usize> {
        (0..self.creatures.len())
            .filter(|&i| self.creatures[i].mob.state.fights() && !self.creatures[i].mob.is_dead() && flat(self.creatures[i].mob.pos - feet).length() <= 12.0)
            .min_by(|&a, &b| flat(self.creatures[a].mob.pos - feet).length().total_cmp(&flat(self.creatures[b].mob.pos - feet).length()))
    }

    /// The level and the type of creature `i`, if it is a hostile one that is alive.
    pub fn enemy_info(&self, i: usize) -> Option<(u32, u32)> {
        self.enemy_at(i).map(|_| (self.creatures[i].mob.def.level, self.creatures[i].mob.def.kind))
    }

    /// A spell hurts every hostile creature within `radius` yards of `centre`, each for what `roll` gives (by its level and type). Returns where above
    /// each the damage shows, and how much it was.
    pub fn spell_area(&mut self, centre: Vec3, radius: f32, mut roll: impl FnMut(u32, u32) -> f32) -> Vec<(Vec3, f32)> {
        let near: Vec<usize> = (0..self.creatures.len()).filter(|&i| self.enemy_at(i).is_some_and(|p| flat(p - centre).length() <= radius)).collect();
        near.into_iter().filter_map(|i| {
            let damage = roll(self.creatures[i].mob.def.level, self.creatures[i].mob.def.kind);
            self.spell_hit(i, damage).map(|at| (at, damage))
        }).collect()
    }

    pub fn clear_target(&mut self) {
        self.target = None;
    }

    /// Select a creature (for previews).
    pub fn set_target(&mut self, creature: usize) {
        self.target = (creature < self.creatures.len()).then_some(creature);
    }

    /// Tab: select the nearest living hostile creature ahead of `from` (facing `yaw`) within range; again, the next
    /// nearest. Nothing changes when there is none.
    pub fn cycle_target(&mut self, from: Vec3, yaw: f32) {
        let ahead = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        let mut near: Vec<(f32, usize)> = (0..self.creatures.len())
            .filter_map(|i| {
                let m = &self.creatures[i].mob;
                let to = flat(m.pos - from);
                let d = to.length();
                (m.def.attackable() && !m.is_dead() && d <= TARGET_RANGE && (d < 2.0 || to.normalize().dot(ahead) > 0.0)).then_some((d, i))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let next = match self.target.and_then(|t| near.iter().position(|&(_, i)| i == t)) {
            Some(at) => near[(at + 1) % near.len()].1,
            None => match near.first() {
                Some(&(_, i)) => i,
                None => return,
            },
        };
        self.target = Some(next);
    }

    /// What the target frame shows. A target that went out of sight (further than `from` can see) is let go.
    pub fn target_view(&mut self, from: Vec3) -> Option<TargetView> {
        let i = self.target?;
        let c = self.creatures.get(i)?;
        if flat(c.mob.pos - from).length() > TARGET_LOSE {
            self.target = None;
            return None;
        }
        let m = &c.mob;
        Some(TargetView { id: i, name: m.def.name.clone(), level: m.def.level, fraction: (m.health / m.def.health).clamp(0.0, 1.0), dead: m.is_dead(), hostile: m.def.is_hostile(), neutral: m.def.is_neutral() })
    }

    /// A camera looking at the head of a creature and the entities to draw, for the portrait of the target frame.
    pub fn portrait_shot(&self, creature: usize) -> Option<(Camera, Vec<usize>)> {
        let c = self.creatures.get(creature)?;
        let (distance, height, entities) = match (&c.visual, &c.mob.def.body) {
            (Visual::Person(p), _) => (1.25 * p.tall(), 1.84 * p.tall(), p.worn.entities()),
            (Visual::Beast { entity, .. }, Body::Beast { scale, .. }) => (1.9 * scale, 0.85 * scale, vec![*entity]),
            (Visual::Beast { entity, .. }, _) => (1.9, 0.85, vec![*entity]),
            (Visual::None, _) => return None,
        };
        // In front of the creature, looking back at it.
        let front = c.mob.yaw + std::f32::consts::PI;
        let (sf, cf) = (front.sin(), front.cos());
        let mut camera = Camera::new(c.mob.pos + Vec3::new(-sf * distance, height, cf * distance), front, 0.0);
        (camera.fov_y, camera.aspect, camera.near, camera.far) = (30f32.to_radians(), 1.0, 0.1, 40.0);
        Some((camera, entities))
    }

    /// Take every creature out of the world, giving its entities back: the world is made again from new content.
    pub fn retire(self, renderer: &mut Renderer) {
        for c in self.creatures {
            match c.visual {
                Visual::Beast { entity, .. } => renderer.remove_entity(entity),
                Visual::Person(p) => p.retire(renderer),
                Visual::None => {}
            }
            c.marks.into_iter().flatten().for_each(|e| renderer.remove_entity(e));
        }
    }

    /// Say which quest items are wanted by the quests being worked on (every frame, before anything dies).
    pub fn set_wanted(&mut self, wanted: Vec<usize>) {
        self.wanted = wanted;
    }

    /// The kinds of the creatures that died since the last call (for the kill goals of quests).
    pub fn take_deaths(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.deaths)
    }

    /// Say which quest mark each creature (by its kind) shows over its head now, if any, and where the marks are looked at
    /// from (they turn to face the viewer).
    pub fn set_markers(&mut self, viewer: Vec3, marker: impl Fn(usize) -> Option<Marker>) {
        self.viewer = viewer;
        for c in &mut self.creatures {
            c.want = if c.mob.is_dead() { None } else { marker(c.mob.kind) };
        }
    }

    /// The nearest creature within `range` yards of `from` that passes `test`.
    fn nearest(&self, from: Vec3, range: f32, test: impl Fn(&Mob) -> bool) -> Option<usize> {
        let dist = |c: &Creature| flat(c.mob.pos - from).length();
        (0..self.creatures.len()).filter(|&i| test(&self.creatures[i].mob) && dist(&self.creatures[i]) <= range).min_by(|&a, &b| dist(&self.creatures[a]).total_cmp(&dist(&self.creatures[b])))
    }

    /// The nearest corpse within `range` yards of `from` that still has something to loot.
    pub fn lootable_near(&self, from: Vec3, range: f32) -> Option<usize> {
        self.nearest(from, range, |m| m.is_dead() && m.loot.is_some())
    }

    /// The nearest living creature within `range` yards of `from` that can be talked to (one that is not hostile).
    pub fn talkable_near(&self, from: Vec3, range: f32) -> Option<usize> {
        self.nearest(from, range, |m| !m.def.attackable() && !m.is_dead())
    }

    pub fn def(&self, creature: usize) -> Option<&'static CreatureDef> {
        self.creatures.get(creature).map(|c| c.mob.def)
    }

    /// Turn a creature to face `target` (feet position).
    pub fn face(&mut self, creature: usize, target: Vec3) {
        if let Some(c) = self.creatures.get_mut(creature) {
            let to = flat(target - c.mob.pos);
            if to.length() > 0.01 {
                c.mob.yaw = to.x.atan2(-to.z);
            }
        }
    }

    pub fn loot(&self, creature: usize) -> Option<&Loot> {
        self.creatures.get(creature)?.mob.loot.as_ref()
    }

    pub fn corpse_pos(&self, creature: usize) -> Option<Vec3> {
        self.creatures.get(creature).map(|c| c.mob.pos)
    }

    /// Leave `loot` on the corpse (what is left after looting); an empty one empties the corpse.
    pub fn set_loot(&mut self, creature: usize, loot: Loot) {
        if let Some(c) = self.creatures.get_mut(creature).filter(|c| c.mob.is_dead()) {
            c.mob.loot = (!loot.is_empty()).then_some(loot);
        }
    }

    /// Whether any creature is after the player (chasing, biting or flinching): the player is in combat.
    pub fn engaged(&self) -> bool {
        self.creatures.iter().any(|c| c.mob.state.fights())
    }

    /// Who is after the player, for the log: the kind, the state and how far from the player.
    pub fn engaged_by(&self, player: Vec3) -> Vec<String> {
        self.creatures.iter().filter(|c| c.mob.state.fights()).map(|c| format!("{} {:?} at {:.1} yards (home {:.1})", c.mob.def.name, c.mob.state, flat(player - c.mob.pos).length(), flat(player - c.mob.home).length())).collect()
    }

    /// Where a point is on a `size` screen seen through `camera` (pixels), if it is in front of it.
    fn on_screen(camera: &Camera, size: (u32, u32), at: Vec3) -> Option<(f32, f32)> {
        let clip = camera.view_proj_relative() * (at - camera.pos).extend(1.0);
        (clip.w > 0.1).then(|| ((clip.x / clip.w * 0.5 + 0.5) * size.0 as f32, (0.5 - clip.y / clip.w * 0.5) * size.1 as f32))
    }

    /// The creature under the pointer at `pos` (pixels) on a `size` screen seen through `camera`, and the pointer that suits it:
    /// the sword over an enemy, a bag over a corpse with loot or a vendor, a speech bubble over someone with something to say.
    pub fn pick(&self, camera: &Camera, size: (u32, u32), pos: (f32, f32)) -> Option<(usize, Kind)> {
        (0..self.creatures.len())
            .filter_map(|i| {
                let c = &self.creatures[i];
                let m = &c.mob;
                if matches!(c.visual, Visual::None) {
                    return None;
                }
                let kind = if m.is_dead() {
                    m.loot.is_some().then_some(Kind::LootAll)?
                } else if m.def.attackable() {
                    Kind::Attack
                } else if !m.def.vendor.is_empty() {
                    Kind::Buy
                } else {
                    m.def.gossip.is_some().then_some(Kind::Speak)?
                };
                // About as tall as it is: a person two yards, a beast by its scale, a corpse lying low.
                let (height, width) = match (&c.visual, &m.def.body) {
                    (Visual::Person(p), _) => (HUMAN_HEIGHT * p.tall(), 0.25),
                    (_, Body::Beast { scale, .. }) => (*scale, 0.8),
                    _ => (1.0, 0.8),
                };
                let [x0, y0, x1, y1] = screen_box(camera, size, m.pos, if m.is_dead() { height * 0.4 } else { height }, width)?;
                (pos.0 >= x0 && pos.0 <= x1 && pos.1 >= y0 && pos.1 <= y1).then(|| ((m.pos - camera.pos).length(), i, kind))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i, kind)| (i, kind))
    }

    /// The most any creature is alert to the player at `feet` (see `Mob::alert`).
    pub fn alert(&self, feet: Vec3, level: u32) -> f32 {
        self.creatures.iter().map(|c| c.mob.alert(feet, level)).fold(0.0, f32::max)
    }

    /// The names floating over the living creatures near the camera: red for hostile ones, yellow for neutral ones, green for friendly ones.
    pub fn labels(&self, camera: &Camera, size: (u32, u32)) -> Vec<Label> {
        self.creatures
            .iter()
            .filter(|c| !c.mob.is_dead() && !matches!(c.visual, Visual::None) && (c.mob.pos - camera.pos).length() <= NAME_RANGE)
            .filter_map(|c| {
                let height = if let Visual::Person(p) = &c.visual { NAME_HEIGHT.1 * p.tall() } else { NAME_HEIGHT.0 };
                let (x, y) = Self::on_screen(camera, size, c.mob.pos + Vec3::Y * height)?;
                let color = if c.mob.def.is_hostile() { [1.0, 0.1, 0.1, 1.0] } else if c.mob.def.is_neutral() { [1.0, 0.9, 0.1, 1.0] } else { [0.1, 1.0, 0.1, 1.0] };
                Some(Label { text: c.mob.def.name.clone(), color, x, y })
            })
            .collect()
    }

    /// Health bars over the hostile creatures that are hurt, drawn on a `size` screen seen through `camera`.
    pub fn draw_plates(&self, ui: &mut Ui, camera: &Camera, size: (u32, u32)) {
        let h = size.1 as f32;
        for c in self.creatures.iter().filter(|c| !c.mob.is_dead() && c.mob.health < c.mob.def.health) {
            let Some((x, y)) = Self::on_screen(camera, size, c.mob.pos + Vec3::Y * 1.7) else { continue };
            let (bar_w, bar_h) = (60.0 * h / 768.0, 7.0 * h / 768.0);
            let fill = (c.mob.health / c.mob.def.health).clamp(0.0, 1.0);
            let mut quad = |rect: [f32; 4], color: [f32; 4]| ui.quad(Quad { texture: WHITE, rect, uv: FULL_UV, color, additive: false });
            quad([x - bar_w / 2.0 - 1.0, y - 1.0, bar_w + 2.0, bar_h + 2.0], [0.0, 0.0, 0.0, 0.85]);
            quad([x - bar_w / 2.0, y, bar_w * fill, bar_h], [0.2 + 0.6 * (1.0 - fill), 0.8 * fill + 0.1, 0.1, 1.0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind() -> usize {
        content::creature_index("wolf")
    }

    fn def() -> &'static CreatureDef {
        content::creature("wolf")
    }

    fn flat_ground(_: f32, _: f32, _: f32) -> Option<f32> {
        Some(0.0)
    }

    fn open(_: Vec3, _: Vec3) -> bool {
        false
    }

    /// Run `seconds` at 60 fps against a player standing at `target`; collects the damage dealt.
    fn run(mob: &mut Mob, seconds: f32, target: Vec3) -> Vec<f32> {
        (0..(seconds * 60.0) as usize).filter_map(|_| mob.update(1.0 / 60.0, target, flat_ground, open)).collect()
    }

    #[test]
    fn a_wolf_ignores_a_distant_player_then_chases_bites_and_keeps_biting() {
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        assert!(run(&mut wolf, 3.0, Vec3::new(30.0, 0.0, 0.0)).is_empty() && wolf.state == State::Idle);
        // A player 10 yards east: it turns to face east and runs to him (6.94 yd/s: about 1.2 s to get in range).
        let player = Vec3::new(10.0, 0.0, 0.0);
        run(&mut wolf, 0.2, player);
        assert_eq!(wolf.state, State::Chase);
        assert!((wolf.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "yaw {}", wolf.yaw);
        let bites = run(&mut wolf, 6.0, player);
        assert!((wolf.pos - player).length() < BITE_RANGE, "stopped at {:?}", wolf.pos);
        // Over 6 seconds: a bite every 2.1 s (1.5 s animation and a pause).
        assert!(bites.len() >= 2 && bites.iter().all(|&d| d == def().bite_damage), "{bites:?}");
    }

    #[test]
    fn a_wolf_does_not_walk_through_a_wall_but_slides_along_it() {
        // A wall across x = 4, and a player behind it, 10 yards east and 3 north.
        let wall = |_: Vec3, to: Vec3| to.x > 4.0;
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        let player = Vec3::new(10.0, 0.0, -3.0);
        for _ in 0..600 {
            wolf.update(1.0 / 60.0, player, flat_ground, wall);
        }
        assert!(wolf.pos.x <= 4.0, "went through: {:?}", wolf.pos);
        // Along the wall it went towards the player's side.
        assert!(wolf.pos.z < -2.0, "stuck at {:?}", wolf.pos);
    }

    #[test]
    fn a_bite_misses_when_the_player_has_stepped_away_and_a_wolf_gives_up_far_away() {
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.state = State::Bite(0.0, false);
        // Moves 5 yards off while the teeth are coming: no damage.
        assert!(run(&mut wolf, 1.6, Vec3::new(5.0, 0.0, 0.0)).is_empty());
        wolf.state = State::Chase;
        run(&mut wolf, 0.1, Vec3::new(50.0, 0.0, 0.0));
        assert!(matches!(wolf.state, State::Return(_)));
    }

    /// A wound that bleeds, a stun that holds it, a slow that makes its time run slower.
    #[test]
    fn what_is_put_on_a_creature_hurts_holds_or_slows_it() {
        let far = Vec3::new(100.0, 0.0, 0.0);
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        let full = wolf.health;
        wolf.afflict(Status::Over { amount: 3.0, period: 1.0, seconds: 5.0 });
        run(&mut wolf, 5.5, far);
        assert!((full - wolf.health - 15.0).abs() < 1e-3, "five ticks of three: {}", full - wolf.health);
        // A stun: it stands where it is for the seconds, then chases.
        wolf.state = State::Chase;
        wolf.pos = Vec3::new(10.0, 0.0, 0.0);
        wolf.afflict(Status::Stun(1.0));
        run(&mut wolf, 0.9, Vec3::ZERO);
        assert_eq!(wolf.pos.x, 10.0);
        run(&mut wolf, 0.6, Vec3::ZERO);
        assert!(wolf.pos.x < 10.0);
        // A slow to a share of the pace: it covers that share of the ground.
        let (mut quick, mut slow) = (Mob::new(kind(), Vec3::new(30.0, 0.0, 0.0), 0.0), Mob::new(kind(), Vec3::new(30.0, 0.0, 0.0), 0.0));
        (quick.state, slow.state) = (State::Chase, State::Chase);
        slow.afflict(Status::Slow(0.5, 10.0));
        run(&mut quick, 1.0, Vec3::ZERO);
        run(&mut slow, 1.0, Vec3::ZERO);
        assert!(((30.0 - slow.pos.x) / (30.0 - quick.pos.x) - 0.5).abs() < 0.1, "{} {}", slow.pos.x, quick.pos.x);
        // Damage over time can kill, and says so once.
        let mut weak = Mob::new(kind(), Vec3::ZERO, 0.0);
        weak.health = 2.0;
        weak.afflict(Status::Over { amount: 5.0, period: 1.0, seconds: 5.0 });
        run(&mut weak, 1.5, far);
        assert!(weak.is_dead() && std::mem::take(&mut weak.slain_over) && !weak.slain_over);
    }

    #[test]
    fn a_wolf_that_gave_up_waits_then_runs_home_and_heals_unless_hit_again() {
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.pos = Vec3::new(30.0, 0.0, 0.0);
        wolf.health = 10.0;
        wolf.state = State::Chase;
        let far = Vec3::new(100.0, 0.0, 0.0);
        run(&mut wolf, 0.1, far);
        assert!(matches!(wolf.state, State::Return(_)));
        // It stands still for a while, hurt...
        run(&mut wolf, RETURN_WAIT - 0.5, far);
        assert_eq!((wolf.pos.x, wolf.health), (30.0, 10.0));
        // ...a hit in that time turns it back on the attacker...
        let mut hit = Mob::new(kind(), Vec3::ZERO, 0.0);
        hit.state = State::Return(1.0);
        hit.hurt(1.0);
        assert!(matches!(hit.state, State::Flinch(_)));
        // ...otherwise it runs home and is whole again.
        run(&mut wolf, 8.0, far);
        assert_eq!((wolf.state, wolf.health), (State::Idle, def().health));
        assert!(wolf.pos.x < 0.3, "{:?}", wolf.pos);
    }

    #[test]
    fn hits_flinch_then_kill_and_the_wolf_returns_after_a_while() {
        let mut wolf = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.0);
        wolf.hurt(12.0);
        assert_eq!((wolf.state, wolf.health), (State::Flinch(0.0), 38.0));
        // It flinches (no movement), then chases the one who hit it from far away... within the leash.
        run(&mut wolf, 0.3, Vec3::new(20.0, 0.0, 0.0));
        assert_eq!(wolf.pos.x, 3.0);
        run(&mut wolf, 0.3, Vec3::new(20.0, 0.0, 0.0));
        assert_eq!(wolf.state, State::Chase);
        (0..4).for_each(|_| wolf.hurt(12.0));
        assert!(wolf.is_dead() && wolf.health < 0.0);
        // A corpse neither moves nor bites, and hits on it do nothing.
        assert!(run(&mut wolf, 10.0, Vec3::new(3.5, 0.0, 0.0)).is_empty() && wolf.is_dead());
        wolf.hurt(5.0);
        // After the respawn time it stands at home again with full health.
        run(&mut wolf, def().respawn - 9.0, Vec3::new(500.0, 0.0, 0.0));
        assert_eq!((wolf.state, wolf.health, wolf.pos), (State::Idle, def().health, Vec3::new(3.0, 0.0, 0.0)));
    }

    #[test]
    fn a_saved_creature_comes_back_as_it_was_and_only_to_its_own_spawn() {
        let mut wolf = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.5);
        wolf.hurt(def().health + 1.0);
        wolf.pos = Vec3::new(4.0, 0.0, 1.0);
        wolf.state = State::Dead(12.5);
        let saved: MobSave = ron::from_str(&ron::to_string(&wolf.snapshot()).unwrap()).unwrap();
        let mut fresh = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.0);
        assert!(fresh.restore(&saved));
        assert_eq!(fresh.snapshot(), wolf.snapshot());
        assert!(fresh.is_dead() && fresh.pos == Vec3::new(4.0, 0.0, 1.0));
        // Another spawn (a different home) is not the same creature.
        assert!(!Mob::new(kind(), Vec3::new(50.0, 0.0, 0.0), 0.0).restore(&saved));
    }

    #[test]
    fn a_swing_hits_what_is_in_front_and_in_reach_only() {
        // The player at the origin faces north (-Z).
        let hit = |at: Vec3| Mob::new(kind(), at, 0.0).struck_by(Vec3::ZERO, 0.0, 3.5, 12.0);
        assert!(hit(Vec3::new(0.0, 0.0, -3.0)));
        assert!(!hit(Vec3::new(0.0, 0.0, -4.0)), "out of reach");
        assert!(!hit(Vec3::new(0.0, 0.0, 3.0)), "behind");
        assert!(!hit(Vec3::new(3.0, 0.0, -0.5)), "to the side");
        assert!(hit(Vec3::new(1.5, 0.0, -2.5)), "within the cone");
        let mut dead = Mob::new(kind(), Vec3::new(0.0, 0.0, -2.0), 0.0);
        dead.state = State::Dead(0.0);
        assert!(!dead.struck_by(Vec3::ZERO, 0.0, 3.5, 12.0));
    }

    #[test]
    fn a_wolf_cries_when_it_notices_snarls_and_bites_is_wounded_and_dies() {
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        let player = Vec3::new(10.0, 0.0, 0.0);
        let mut heard = Vec::new();
        for _ in 0..(5.0 * 60.0) as usize {
            wolf.update(1.0 / 60.0, player, flat_ground, open);
            heard.append(&mut wolf.sounds);
        }
        // Paws hit the ground while it runs (8 yards: three or so steps).
        let steps = heard.iter().filter(|&&s| s == Cry::Footstep).count();
        assert!((3..=4).contains(&steps), "{steps} footsteps");
        heard.retain(|&s| s != Cry::Footstep);
        // Aggro once, then a snarl and the bite's impact for each bite.
        assert_eq!(heard[..3], [Cry::Aggro, Cry::Attack, Cry::Bite]);
        assert_eq!(heard.iter().filter(|&&s| s == Cry::Aggro).count(), 1);
        wolf.hurt(12.0);
        assert_eq!(wolf.sounds.pop(), Some(Cry::Wound));
        (0..4).for_each(|_| wolf.hurt(12.0));
        assert_eq!(wolf.sounds.last(), Some(&Cry::Death));
        wolf.sounds.clear();
        wolf.hurt(12.0);
        assert!(wolf.sounds.is_empty(), "a corpse makes no sound");
    }

    #[test]
    fn a_killed_wolf_holds_loot_until_it_comes_back() {
        let mut wolf = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.0);
        assert!(wolf.loot.is_none());
        wolf.hurt(12.0);
        assert!(wolf.loot.is_none(), "a living wolf has nothing to loot");
        (0..4).for_each(|_| wolf.hurt(12.0));
        assert!(wolf.is_dead() && wolf.loot.is_some());
        // Hits on a corpse do not roll again.
        let rolled = wolf.loot.clone();
        wolf.hurt(5.0);
        assert_eq!(wolf.loot, rolled);
        run(&mut wolf, def().respawn + 1.0, Vec3::new(500.0, 0.0, 0.0));
        assert!(wolf.loot.is_none() && !wolf.is_dead());
    }

    #[test]
    fn a_corpse_out_of_reach_still_counts_down_to_the_day_it_is_back_and_only_what_changed_is_saved() {
        let mut mob = Mob::new(content::creature_index("c299"), Vec3::new(5.0, 0.0, 5.0), 0.0);
        assert!(!mob.worth_saving(), "one at rest starts afresh");
        mob.health -= 1.0;
        assert!(mob.worth_saving());
        (mob.state, mob.pos) = (State::Dead(0.0), Vec3::new(9.0, 0.0, 9.0));
        mob.tick_dead(mob.respawn - 1.0);
        assert!(matches!(mob.state, State::Dead(_)) && mob.worth_saving());
        mob.tick_dead(2.0);
        assert_eq!((mob.state, mob.pos, mob.health), (State::Idle, Vec3::new(5.0, 0.0, 5.0), mob.def.health));
        assert!(!mob.worth_saving());
    }

    #[test]
    fn an_unbothered_hostile_creature_strolls_near_home_and_rests_between_walks() {
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        let far = Vec3::new(500.0, 0.0, 0.0);
        let (mut walked, mut rested, mut farthest) = (false, false, 0.0f32);
        for _ in 0..(120.0 * 60.0) as usize {
            wolf.update(1.0 / 60.0, far, flat_ground, open);
            walked |= matches!(wolf.state, State::Wander(_));
            rested |= walked && wolf.state == State::Idle;
            farthest = farthest.max(flat(wolf.pos).length());
        }
        assert!(walked && rested, "it walks, then stands again");
        assert!(farthest > 1.0 && farthest <= WANDER_DEFAULT + 0.5, "strolled {farthest} yards from home");
    }

    fn stop(x: f32, z: f32, wait: f32, steps: Vec<(f32, content::Step)>) -> Stop {
        Stop { at: Vec3::new(x, 0.0, z), wait, facing: None, steps }
    }

    /// A guard (a friendly creature) with a patrol, run for `seconds` far from any player; calls `watch` after each step.
    fn patrol(route: Vec<Stop>, seconds: f32, mut watch: impl FnMut(&Mob)) -> Mob {
        let mut guard = Mob::new(content::creature_index("c1642"), Vec3::ZERO, 0.0);
        guard.route = route;
        let far = Vec3::new(500.0, 0.0, 500.0);
        for _ in 0..(seconds * 60.0) as usize {
            guard.update(1.0 / 60.0, far, flat_ground, open);
            watch(&guard);
        }
        guard
    }

    #[test]
    fn a_patrol_walks_its_places_in_turn_standing_and_facing_where_they_ask_and_comes_back_in_its_own_time() {
        // Three places, the second asking for ten seconds standing with the face turned.
        let mut route = vec![stop(10.0, 0.0, 0.0, vec![]), stop(10.0, 10.0, 10.0, vec![]), stop(0.0, 10.0, 0.0, vec![])];
        route[1].facing = Some(1.0);
        let (mut reached, mut stood) = ([false; 3], 0.0f32);
        let places: Vec<Vec3> = route.iter().map(|r| r.at).collect();
        patrol(route, 120.0, |g| {
            for (r, place) in reached.iter_mut().zip(&places) {
                *r |= flat(g.pos - *place).length() < 0.5;
            }
            if flat(g.pos - places[1]).length() < 0.5 && g.state == State::Idle {
                stood += 1.0 / 60.0;
                assert_eq!(g.yaw, 1.0, "it turns to the heading of the place while it stands");
            }
        });
        assert_eq!(reached, [true; 3], "every place of the patrol is reached");
        assert!(stood > 9.0, "it stands at the second place: {stood} s");
        // Its own respawn time is the one it waits.
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.respawn = 20.0;
        wolf.state = State::Dead(0.0);
        run(&mut wolf, 19.0, Vec3::new(500.0, 0.0, 500.0));
        assert!(wolf.is_dead());
        run(&mut wolf, 2.0, Vec3::new(500.0, 0.0, 500.0));
        assert!(!wolf.is_dead());
    }

    #[test]
    fn the_scripts_of_the_places_make_it_run_hold_an_animation_change_its_look_and_stay_where_it_is() {
        use content::Step;
        let wolf = content::creature_index("wolf");
        // The first place: run from here on at once, hold animation 136 after half a second, and look like a wolf after a second. The second: stay.
        let route = vec![stop(10.0, 0.0, 2.0, vec![(0.0, Step::Run(true)), (0.5, Step::Emote(136)), (1.0, Step::Morph("wolf".into()))]), stop(30.0, 0.0, 0.0, vec![(0.0, Step::Idle)])];
        let (mut emote_seen, mut look_seen, mut fastest) = (None, None, 0.0f32);
        let mut last = Vec3::ZERO;
        let guard = patrol(route, 40.0, |g| {
            if g.state == State::Idle && flat(g.pos - Vec3::new(10.0, 0.0, 0.0)).length() < 0.5 {
                emote_seen = emote_seen.or(g.emote);
                look_seen = look_seen.or((g.look == wolf).then_some(true));
            }
            fastest = fastest.max(flat(g.pos - last).length() * 60.0);
            last = g.pos;
        });
        assert_eq!((emote_seen, look_seen), (Some(136), Some(true)));
        assert!(fastest > WALK_SPEED * 1.5, "it ran between the places: {fastest}");
        assert!(guard.stopped && flat(guard.pos - Vec3::new(30.0, 0.0, 0.0)).length() < 0.5 && guard.state == State::Idle, "it stays at the last place");
        // Back from the dead it starts again, walking, as it looked.
        let mut guard = guard;
        guard.state = State::Dead(guard.respawn);
        run(&mut guard, 1.0, Vec3::new(500.0, 0.0, 500.0));
        assert_eq!((guard.next, guard.run, guard.stopped, guard.emote, guard.look), (0, false, false, None, guard.kind));
    }

    #[test]
    fn a_script_can_have_it_stroll_round_a_place_for_a_while_before_the_patrol_goes_on() {
        let route = vec![stop(10.0, 0.0, 0.0, vec![(0.0, content::Step::Roam(6.0, 20.0))]), stop(40.0, 0.0, 0.0, vec![])];
        let (mut strayed, mut left_at) = (0.0f32, None);
        patrol(route, 60.0, |g| {
            if g.roam.is_some() {
                strayed = strayed.max(flat(g.pos - Vec3::new(10.0, 0.0, 0.0)).length());
            } else if g.next == 0 || left_at.is_some() {
            } else {
                left_at = Some(());
            }
        });
        assert!(strayed > 1.0 && strayed <= 6.5, "it strolled within six yards of the place: {strayed}");
        assert!(left_at.is_some(), "and then went on");
    }

    #[test]
    fn how_far_along_its_patrol_a_creature_is_comes_back_with_the_save() {
        let route = vec![stop(10.0, 0.0, 30.0, vec![(0.0, content::Step::Run(true))]), stop(10.0, 10.0, 0.0, vec![]), stop(0.0, 10.0, 0.0, vec![])];
        let guard = patrol(route.clone(), 20.0, |_| {});
        assert_eq!((guard.next, guard.run), (1, true), "at the first place, bound for the second");
        let saved: MobSave = ron::from_str(&ron::to_string(&guard.snapshot()).unwrap()).unwrap();
        let mut back = Mob::new(content::creature_index("c1642"), Vec3::ZERO, 0.0);
        back.route = route;
        assert!(back.restore(&saved));
        assert_eq!((back.next, back.run), (1, true));
    }

    #[test]
    fn a_stroll_is_one_to_three_walks_within_the_spawns_own_distance_and_zero_stands_still() {
        let far = Vec3::new(500.0, 0.0, 0.0);
        let (mut wolf, mut still) = (Mob::new(kind(), Vec3::ZERO, 0.0), Mob::new(kind(), Vec3::ZERO, 0.0));
        (wolf.wander, still.wander) = (12.0, 0.0);
        let (mut most_legs, mut farthest) = (0, 0.0f32);
        for _ in 0..(600.0 * 60.0) as usize {
            wolf.update(1.0 / 60.0, far, flat_ground, open);
            still.update(1.0 / 60.0, far, flat_ground, open);
            most_legs = most_legs.max(wolf.legs);
            farthest = farthest.max(flat(wolf.pos).length());
            assert!(still.state == State::Idle, "wander 0 never walks");
        }
        assert_eq!(most_legs, 3, "up to three walks in a row");
        assert!(farthest > 6.0 && farthest <= 12.5, "strolled {farthest} yards from home");
    }

    #[test]
    fn a_friendly_creature_stands_still_cannot_be_hurt_and_can_be_talked_to() {
        let guard = content::creature_index("deputy_willem");
        let mut mob = Mob::new(guard, Vec3::new(2.0, 0.0, 0.0), 0.0);
        assert!(run(&mut mob, 5.0, Vec3::new(3.0, 0.0, 0.0)).is_empty());
        assert_eq!((mob.state, mob.pos), (State::Idle, Vec3::new(2.0, 0.0, 0.0)));
        assert!(!mob.struck_by(Vec3::new(2.0, 0.0, 3.0), 0.0, 3.5, 12.0) && mob.health == content::creature("deputy_willem").health);
        assert!(!mob.def.hostile && mob.def.title.is_some());
    }

    #[test]
    fn a_neutral_creature_ignores_the_player_until_hit_then_fights_back() {
        let kind = content::creature_index("c299"); // Young Wolf
        let mut wolf = Mob::new(kind, Vec3::ZERO, 0.0);
        assert!(wolf.def.neutral && !wolf.def.hostile && wolf.def.attackable());
        assert!(run(&mut wolf, 3.0, Vec3::new(3.0, 0.0, 0.0)).is_empty() && !matches!(wolf.state, State::Chase | State::Bite(..)), "no aggro on sight");
        assert!(wolf.struck_by(wolf.pos + Vec3::Z * 2.0, 0.0, 3.5, 1.0), "it can be fought");
        let beside = wolf.pos + Vec3::X;
        let bitten = run(&mut wolf, 3.0, beside);
        assert!(!bitten.is_empty(), "it bites back");
    }

    /// The quest marks bob up and down in their stand animation, and the "?" ones turn to face the viewer (their one bone
    /// is a billboard locked to the vertical axis, 0x40), which the game has to do itself.
    #[test]
    fn the_quest_mark_models_bob_and_the_question_marks_face_the_viewer() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        for path in MARKER_MODELS {
            let m2 = M2::parse(&mount.read(path).unwrap()).unwrap();
            let seq = m2.sequences.iter().position(|q| q.id == STAND).unwrap();
            let start = m2.sequences[seq].start as f32;
            let at = |t: f32| anim::bone_matrices(&m2, seq, start + t, t)[0];
            assert!(at(0.0).abs_diff_eq(at(1533.0), 1e-2) && !at(0.0).abs_diff_eq(at(800.0), 1e-3), "{path} does not bob");
            assert_eq!(m2.bones[0].flags & 0x40 != 0, path.contains("Question"), "{path}");
        }
    }

    #[test]
    fn a_sneaking_player_is_noticed_only_from_close_by() {
        let range = aggro_range(content::creature("wolf").level, 1);
        let near_but_hidden = Vec3::new(range * SNEAK_AGGRO + 2.0, 0.0, 0.0);
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.hidden = true;
        assert!(wolf.alert(Vec3::new(range + 1.0, 0.0, 0.0), 1) == 0.0 && wolf.alert(near_but_hidden, 1) > 0.0 && wolf.alert(near_but_hidden, 1) < 1.0, "the eye opens as it comes close");
        assert!(run(&mut wolf, 1.0, near_but_hidden).is_empty() && wolf.state == State::Idle, "not noticed while sneaking");
        wolf.hidden = false;
        run(&mut wolf, 0.1, near_but_hidden);
        assert_eq!(wolf.state, State::Chase, "noticed once he stands up");
        // Sneaking right up to it works only so far.
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.hidden = true;
        run(&mut wolf, 0.1, Vec3::new(range * SNEAK_AGGRO - 1.0, 0.0, 0.0));
        assert_eq!(wolf.state, State::Chase);
    }

    #[test]
    fn a_wolf_follows_the_ground() {
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.update(0.016, Vec3::new(100.0, 0.0, 0.0), |x, _, _| Some(x * 0.5 + 2.0), open);
        assert_eq!(wolf.pos.y, 2.0);
    }

    /// Run `seconds` at 60 fps against the player `watch` describes; everything the mob did, in order.
    fn acts_of(mob: &mut Mob, seconds: f32, watch: &Watch) -> Vec<Act> {
        (0..(seconds * 60.0) as usize).flat_map(|_| mob.step(1.0 / 60.0, watch, 0, &[], flat_ground, open)).collect()
    }

    fn casts(acts: &[Act]) -> Vec<(u32, Aim)> {
        acts.iter().filter_map(|a| if let Act::Cast(c) = a { Some((c.spell, c.aim)) } else { None }).collect()
    }

    #[test]
    fn a_caster_stands_back_buffs_itself_and_casts_its_bolt_with_the_time_and_mana_it_takes() {
        // Defias Rogue Wizard: ranged mode at 35 yards, Frost Armor (12544) on itself, Frostbolt (13322) as its main spell, mana for many.
        let mut mage = Mob::new(content::creature_index("c474"), Vec3::ZERO, 0.0);
        let mana = mage.mana;
        let player = Vec3::new(12.0, 0.0, 0.0);
        let acts = acts_of(&mut mage, 12.0, &Watch::at(player));
        let all = casts(&acts);
        assert!(all.contains(&(13322, Aim::Victim)), "it casts its bolt: {all:?}");
        // It stands where it is: twelve yards from the player is well inside its ranged distance.
        assert!(mage.pos.length() < 0.5, "it walked to {:?}", mage.pos);
        // A bolt takes its cast time: the begin comes well before the cast.
        let begin = acts.iter().position(|a| matches!(a, Act::Begin(13322))).expect("the cast begins");
        let gone = acts.iter().position(|a| matches!(a, Act::Cast(c) if c.spell == 13322)).expect("and goes off");
        assert!(gone > begin, "begin {begin}, cast {gone}");
        assert!(mage.mana < mana, "casting costs mana");
        // Frost Armor went on it, once (the script casts it while it is missing).
        assert!(all.contains(&(12544, Aim::Itself)) && mage.has_aura(12544), "{all:?}");
        assert_eq!(all.iter().filter(|c| c.0 == 12544).count(), 1);
    }

    #[test]
    fn a_proximity_caster_backs_away_from_a_player_who_comes_close_and_a_wounded_one_runs_for_help() {
        let mut mage = Mob::new(content::creature_index("c474"), Vec3::new(20.0, 0.0, 0.0), 0.0);
        let near = Watch::at(Vec3::new(18.0, 0.0, 0.0));
        // Two yards from him, the mage steps back (ranged mode 2) before it casts its bolt.
        acts_of(&mut mage, 0.2, &near);
        assert!(matches!(mage.state, State::Kite(_)), "{:?} at {:?}", mage.state, mage.pos);
        acts_of(&mut mage, 1.0, &near);
        assert!(mage.pos.x > 21.0, "it backed away to {:?}", mage.pos);
        // Hurt to a tenth of its health, it runs (Flee at 15%) instead of fighting on.
        let mut seer = Mob::new(content::creature_index("c1397"), Vec3::ZERO, 0.0);
        let player = Watch::at(Vec3::new(10.0, 0.0, 0.0));
        acts_of(&mut seer, 0.5, &player);
        seer.health = seer.def.health * 0.1;
        acts_of(&mut seer, 0.2, &player);
        assert!(matches!(seer.state, State::Flee(..)), "{:?}", seer.state);
        // It runs away from the player, and after seven seconds it turns back on him.
        acts_of(&mut seer, 2.0, &player);
        assert!(seer.pos.x < -5.0, "ran to {:?}", seer.pos);
        acts_of(&mut seer, FLEE_TIME, &player);
        assert!(!matches!(seer.state, State::Flee(..)), "{:?}", seer.state);
    }

    #[test]
    fn an_archer_shoots_from_range_and_a_creature_out_of_mana_goes_to_melee() {
        // Zanzil Hunter: ranged mode 1 at 24 yards, Shoot (a ranged attack: no stop to cast, then a short pause).
        let mut hunter = Mob::new(content::creature_index("c1489"), Vec3::ZERO, 0.0);
        let acts = acts_of(&mut hunter, 14.0, &Watch::at(Vec3::new(13.0, 0.0, 0.0)));
        let shots: Vec<_> = casts(&acts).into_iter().filter(|c| c.0 == 10277).collect();
        assert!(shots.len() >= 3, "it keeps shooting: {shots:?}");
        assert!(hunter.pos.length() < 0.5, "it stays back");
        // A caster whose main spell it cannot pay for walks up and fights close; a Frostmane Seer has 171 mana and its bolt takes 90.
        let mut mage = Mob::new(content::creature_index("c474"), Vec3::ZERO, 0.0);
        mage.mana = 1.0;
        let player = Vec3::new(12.0, 0.0, 0.0);
        acts_of(&mut mage, 6.0, &Watch::at(player));
        assert!(flat(mage.pos - player).length() < BITE_RANGE + 0.5, "out of mana it comes to melee: {:?}", mage.pos);
        let mut seer = Mob::new(content::creature_index("c1397"), Vec3::ZERO, 0.0);
        let bolts = casts(&acts_of(&mut seer, 14.0, &Watch::at(player))).iter().filter(|c| c.0 == 9532).count();
        assert_eq!(bolts, 1, "one bolt, and then its mana is gone");
        assert!(flat(seer.pos - player).length() < BITE_RANGE + 0.5, "a Frostmane Seer then fights close: {:?}", seer.pos);
    }

    #[test]
    fn a_dying_creature_casts_its_death_spell_and_a_summoner_calls_its_helpers() {
        let mut ogre = Mob::new(content::creature_index("c2252"), Vec3::ZERO, 0.0);
        acts_of(&mut ogre, 0.1, &Watch::at(Vec3::new(30.0, 0.0, 0.0)));
        ogre.hurt(ogre.def.health + 1.0);
        let acts = acts_of(&mut ogre, 0.1, &Watch::at(Vec3::new(2.0, 0.0, 0.0)));
        assert!(casts(&acts).iter().any(|c| c.0 == 9144), "Crushridge Death goes off: {:?}", casts(&acts));
        // Somebody in the data summons: its script asks for helpers once the fight is on.
        let defs = &content::get().creatures;
        let summoner = defs.iter().position(|d| d.attackable() && d.hostile && d.brain.events.iter().any(|e| e.on == brain::event::TIMER && e.acts.iter().any(|a| a.0 == brain::act::SUMMON))).expect("a summoner");
        let mut mob = Mob::new(summoner, Vec3::ZERO, 0.0);
        let acts = acts_of(&mut mob, 12.0, &Watch::at(Vec3::new(8.0, 0.0, 0.0)));
        assert!(acts.iter().any(|a| matches!(a, Act::Summon { .. })), "{} summoned nobody", mob.def.name);
    }

    #[test]
    fn a_pull_calls_the_others_of_the_faction_and_a_helper_turns_on_the_player() {
        let mut leader = Mob::new(content::creature_index("c2240"), Vec3::ZERO, 0.0);
        let acts = acts_of(&mut leader, 0.5, &Watch::at(Vec3::new(8.0, 0.0, 0.0)));
        assert!(acts.iter().any(|a| matches!(a, Act::Assist(r) if *r > 0.0)), "it calls for help");
        let mut other = Mob::new(content::creature_index("c2240"), Vec3::new(5.0, 0.0, 5.0), 0.0);
        assert!(other.assist() && other.state == State::Chase);
        assert!(!other.assist(), "one that already fights is not called again");
    }

    #[test]
    fn being_hit_holds_a_cast_back_instead_of_stopping_it() {
        let mut seer = Mob::new(content::creature_index("c474"), Vec3::ZERO, 0.0);
        let player = Watch::at(Vec3::new(12.0, 0.0, 0.0));
        acts_of(&mut seer, 0.7, &player);
        let State::Cast(spell, t, total, false) = seer.state else { panic!("{:?}", seer.state) };
        seer.hurt(1.0);
        let State::Cast(s2, t2, total2, false) = seer.state else { panic!("a hit does not break the cast: {:?}", seer.state) };
        assert!(s2 == spell && total2 == total && t2 < t, "pushed back from {t} to {t2}");
    }

    #[test]
    fn an_archer_draws_the_weapon_it_holds_for_its_shot() {
        // Every shooter of the data that is a person with a ranged weapon in its outfit has the animation of that weapon.
        let defs = &content::get().creatures;
        let kinds: Vec<usize> = (0..defs.len()).filter(|&k| matches!(&defs[k].body, Body::Humanoid { outfit, .. } if outfit.iter().any(|&i| matches!(items::get(i).wear, items::Wear::Ranged)))).take(300).collect();
        assert!(!kinds.is_empty(), "somebody holds a bow or a gun");
        for k in kinds {
            let anim = Mob::new(k, Vec3::ZERO, 0.0).shot_anim();
            assert!([crate::animator::ATTACK_BOW, crate::animator::ATTACK_RIFLE, CAST_SPELL].contains(&anim), "{}: {anim}", defs[k].name);
        }
        let mob = Mob::new(content::creature_index("c1489"), Vec3::ZERO, 0.0);
        let (ready, cast) = mob.spell_anims(10277, &SpellAnims::new());
        assert_eq!(ready, cast);
        assert_eq!(mob.spell_anims(13322, &SpellAnims::new()), (READY_SPELL, CAST_SPELL), "a spell with no kit gets the usual stances");
    }

    #[test]
    fn how_far_a_creature_notices_the_player_grows_a_yard_for_each_level_it_is_above_him() {
        assert_eq!(aggro_range(10, 10), 20.0);
        assert_eq!(aggro_range(15, 10), 25.0, "five levels above: five yards more");
        assert_eq!(aggro_range(5, 10), 15.0, "five levels below: five yards less");
        assert_eq!(aggro_range(40, 10), 45.0, "at most 45 yards, at 25 levels above and beyond");
        assert_eq!(aggro_range(1, 60), 5.0, "at least the reach of a blow");
        // A black wolf (level 3) notices a level-1 player from 22 yards, not from 23.
        let mut high = Mob::new(content::creature_index("black_wolf"), Vec3::ZERO, 0.0);
        let level = high.def.level;
        let at = |yards: f32| Watch { level: level - 2, ..Watch::at(Vec3::new(yards, 0.0, 0.0)) };
        high.step(0.1, &at(aggro_range(level, level - 2) + 1.0), 0, &[], flat_ground, open);
        assert_eq!(high.state, State::Idle);
        high.step(0.1, &at(aggro_range(level, level - 2) - 1.0), 0, &[], flat_ground, open);
        assert_eq!(high.state, State::Chase);
    }
}
