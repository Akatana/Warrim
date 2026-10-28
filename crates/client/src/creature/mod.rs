//! Creatures: what stands in the world and is not the player. A hostile beast (a wolf) guards its patch, chases the
//! player once close, strikes, flinches when hit, dies and comes back after a while; a person (a guard) stands where
//! he is and can be talked to. Which kinds exist and where they stand is in `content/` (creatures.ron, spawns.ron).
//! `Mob` is the behaviour (no graphics, so it is tested on its own); `Creatures` draws them: beasts with the
//! model's own animations, people as the same dressed human male the player is.

use crate::anim;
use crate::animator::{Channel, Motion, Rng, SWIM, SWIM_IDLE};
use crate::audio::{Cry, Sfx};
use crate::brain::{self, cast, Aim, Friend, Order, Outcome, Runtime, View};
use crate::camera::Camera;
use crate::cursor::Kind;
use crate::character::{Appearance, Character};
use crate::content::{self, Body, ClassicSpell, CreatureDef};
use crate::items::{self, SLOTS};
use crate::navmesh::{NavMesh, PathFinder};
use crate::worn::Worn;
use crate::loot::Loot;
use crate::quests::Marker;
use crate::models::{ModelCache, ModelCpu, ModelVertex};
use crate::physics::{Bodies, RAGDOLL_REACH};
use crate::ragdoll::Ragdoll;
use crate::renderer::{Gpu, Renderer};
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
const ATTACK_UNARMED: u16 = 16;
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
/// The most a creature steps down (yards): it does not walk off a ledge or into a gap of a floor.
const STEP_DOWN: f32 = 1.0;
const REGEN: f32 = 0.25;
/// A melee attack starts within this distance and lands if the player is still within `MELEE_REACH`, yards.
const MELEE_RANGE: f32 = 2.4;
const MELEE_REACH: f32 = 3.2;
/// Seconds into the 1.5 s melee attack animation at which the teeth land, and when it is over.
const MELEE_HIT: f32 = 0.55;
const MELEE_TIME: f32 = 1.5;
/// Seconds between two strikes, and of a flinch.
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
const POSE_NEAR: f32 = 30.0;
const POSE_FAR: f32 = 70.0;
const MAKE_TIME: Duration = Duration::from_millis(6);
/// The most places the compass and the map are told of for a kind of creature (the nearest).
const MOST_LEADS: usize = 40;
/// The models of the quest marks floating over a creature's head, by `Marker` (the original's own: they spin).
const MARKER_MODELS: [&str; 3] = [r"Interface\Buttons\TalkToMe.m2", r"Interface\Buttons\TalkToMeQuestionMark.m2", r"Interface\Buttons\TalkToMeQuestion_Grey.m2"];
/// How high above a person's feet the mark's model stands (its own origin; it reaches 0.15 to 0.77 above that), over the name.
const MARKER_HEIGHT: f32 = 2.65;
/// Where WoW.exe (`FUN_00608640`) puts the name of a unit: at the model's attachment 18 ("PlayerName"), or else this many yards above the feet for each yard of scale.
const NAME_ATTACHMENT: usize = 18;
const NAME_FALLBACK: f32 = 1.25;
/// How tall the em of a name's font stands in the world, yards (`FUN_006c6e90`): 0.2, or the unit's height / 4 * 1.5 * 0.2 once it is over 4 yards tall.
const NAME_EM: f32 = 0.2;
/// A name is checked against the walls every this many frames, and at most this many creatures are checked in a frame (the rest keep their last answer).
const LOOK_EVERY: u8 = 8;
const LOOKS_PER_FRAME: usize = 6;

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum State {
    /// Standing still (a hostile creature gets bored of it after a while).
    Idle,
    /// Walking to a place near home, given in render space.
    Wander(Vec3),
    Chase,
    /// Seconds into a melee attack, and whether its damage has been dealt.
    Melee(f32, bool),
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
        matches!(self, State::Chase | State::Melee(..) | State::Flinch(_) | State::Cast(..) | State::Flee(..) | State::Kite(_))
    }
}

/// An aura on a creature as the target frame shows it: the name and icon of the spell, its dispel kind (1 magic, 2 curse, 3 disease, 4 poison; 0 none), the seconds left.
#[derive(Clone, Debug)]
pub struct Aura {
    pub name: String,
    pub icon: String,
    pub dispel: u32,
    pub left: f32,
}

/// What is put on a creature: damage over time (the damage of a tick, the seconds between ticks, the seconds it lasts), a stun for some seconds, or a slow
/// that leaves it with a share of its pace for some seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Over { amount: f32, period: f32, seconds: f32 },
    Stun(f32),
    Slow(f32, f32),
    /// Cannot move for some seconds (it still fights what is within its reach).
    Root(f32),
    /// Runs from the player in terror for some seconds.
    Fear(f32),
    /// Asleep for some seconds: any blow wakes it (sleep, sap, shackle, gouge).
    Sleep(f32),
    /// Turned into a sheep for some seconds: harmless, and the first blow turns it back.
    Sheep(f32),
    /// Out of this world for some seconds: it cannot be hurt.
    Banish(f32),
    /// Its blows are `dealt` times as hard and it takes `taken` times the damage, for some seconds (a curse, a shout, an armor torn).
    Weaken { dealt: f32, taken: f32, seconds: f32 },
    /// It swings at this share of its speed (0.75: a quarter slower) for some seconds.
    Tempo(f32, f32),
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
    /// (An ally) its blow lands on what it fights, for this much; and it casts a spell (of the original) at it.
    Strike(f32),
    Spell(u32),
    /// It summons creatures of this entry (for so many milliseconds; 0: until they die).
    Summon { entry: u32, ms: i32 },
    /// It calls the creatures of its faction within so many yards.
    Assist(f32),
}

/// What the game does about a creature's act: the damage of a swing, the start of a spell, its end, a spell gone off (with where on friends it shows).
#[derive(Clone, Debug)]
pub enum Foe {
    /// A swing of this much from a creature of this level.
    Melee(f32, u32),
    Begin { who: usize, spell: u32, at: Vec3, yaw: f32, height: f32 },
    Stop { who: usize },
    Cast { who: usize, cast: Cast, height: f32, allies: Vec<Vec3> },
    /// A spell of a pet goes off at a point `to` of a creature: shown from the pet, with the missile flying to it for `delay` seconds (0: none).
    Bolt { who: usize, spell: u32, from: Vec3, yaw: f32, height: f32, to: Vec3, delay: f32 },
}

/// A cast in the making: who it is for, and its flags.
#[derive(Clone, Copy, Debug)]
struct Pending {
    aim: Aim,
    friend: Option<usize>,
}

pub struct Mob {
    /// Its kind: an index into the content's creatures (health, melee attack damage, respawn time, loot).
    kind: usize,
    def: &'static CreatureDef,
    /// Feet position in render space and heading (radians, 0 = north, like the camera's yaw).
    pub pos: Vec3,
    pub yaw: f32,
    home: Vec3,
    /// The places still to walk through on the way the path finder made, the next one last (empty when there is none), whether that way is a straight line
    /// that no mesh led, whether the way home is walked at once, the path finder (it keeps the way it made), and the seconds until the chase looks at its way again.
    course: Vec<Vec3>,
    /// Where the way last made ends (also when it is walked), the state of the chase, and the seconds until a creature in terror picks its next place.
    dest: Option<Vec3>,
    chase: Chase,
    fear_wait: f32,
    /// It is in water (set from the liquid under it while it moves).
    pub swimming: bool,
    loose: bool,
    fast: bool,
    finder: Option<Box<PathFinder>>,
    recheck: f32,
    /// A stroll's way is made once, at its start.
    planned: bool,
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
    /// Whether the hide was taken from the corpse (a skinning knife).
    pub skinned: bool,
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
    /// The last is the icon of the spell it came from: casting the same spell again renews it instead of adding another.
    over: Vec<(f32, f32, f32, f32, String)>,
    /// What of it the target frame lists.
    debuffs: Vec<Aura>,
    stun: f32,
    slow: (f32, f32),
    /// The share of its attack speed it has left and the seconds that lasts (`Status::Tempo`).
    tempo: (f32, f32),
    /// Seconds it cannot move, seconds it runs in fear, seconds it cannot be hurt, and what weakens it (dealt, taken, seconds left).
    root: f32,
    fear: f32,
    banish: f32,
    weak: (f32, f32, f32),
    /// The stun is a sleep: a blow ends it (and turns a sheep back).
    asleep: bool,
    /// Its pockets were picked.
    picked: bool,
    /// It fights for the player (a pet, a raised corpse, a creature taken over for a while); and its corpse was raised.
    pub ally: Option<Ally>,
    raised: bool,
    /// It was called by a quest to be fought, whatever its kind is.
    forced_hostile: bool,
    slain_over: bool,
    /// The damage of the ticks of damage over time since they were last told (the floating numbers).
    ticked: Vec<f32>,
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
    #[serde(default)]
    skinned: bool,
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
            skinned: self.skinned,
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
        self.skinned = saved.skinned;
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
            course: Vec::new(),
            dest: None,
            chase: Chase::new(),
            fear_wait: 0.0,
            swimming: false,
            loose: false,
            fast: false,
            finder: None,
            recheck: 0.0,
            planned: false,
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
            skinned: false,
            rng: seed,
            stride: 0.0,
            sounds: Vec::new(),
            rt: Some(Runtime::new(&def.brain, seed)),
            mana: def.brain.mana,
            auras: Vec::new(),
            over: Vec::new(),
            debuffs: Vec::new(),
            stun: 0.0,
            slow: (1.0, 0.0),
            tempo: (1.0, 0.0),
            root: 0.0,
            fear: 0.0,
            banish: 0.0,
            weak: (1.0, 1.0, 0.0),
            asleep: false,
            picked: false,
            ally: None,
            raised: false,
            forced_hostile: false,
            slain_over: false,
            ticked: Vec::new(),
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
        (self.course, self.planned) = (Vec::new(), false);
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

}

mod allies;
mod behaviour;
mod combat;
mod creatures;
mod steer;
use steer::{Chase, Mode};
mod person;
mod spells;
#[cfg(test)]
mod tests;

use allies as ally;
pub use allies::{Ally, PetCommand, PetOrders, Stance};
pub use creatures::{screen_box, Creatures, Label, Plate, TargetView};
pub(crate) use person::Person;
use person::*;
