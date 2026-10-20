//! Creatures: what stands in the world and is not the player. A hostile beast (a wolf) guards its patch, chases the
//! player once close, bites, flinches when hit, dies and comes back after a while; a person (a guard) stands where
//! he is and can be talked to. Which kinds exist and where they stand is in `content/` (creatures.ron, spawns.ron).
//! `Mob` is the behaviour (no graphics, so it is tested on its own); `Creatures` draws them: beasts with the
//! model's own animations, people as the same dressed human male the player is.

use crate::anim;
use crate::animator::{Channel, Motion, Rng};
use crate::audio::{Cry, Sfx};
use crate::camera::Camera;
use crate::cursor::Kind;
use crate::character::{Appearance, Character};
use crate::content::{self, Body, CreatureDef};
use crate::items::{self, SLOTS};
use crate::worn::Worn;
use crate::loot::Loot;
use crate::quests::Marker;
use crate::models::{ModelCache, ModelCpu, ModelVertex};
use crate::renderer::{Gpu, Renderer};
use crate::ui::{Quad, Ui, FULL_UV, WHITE};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use wowfmt::m2::M2;

/// Animation ids of the sequences every creature model has.
const STAND: u16 = 0;
const WALK: u16 = 4;
const RUN: u16 = 5;
const DEATH: u16 = 1;
const WOUND: u16 = 9;
const BITE: u16 = 16;

/// How far in front of the player his sword reaches, yards.
pub const SWORD_REACH: f32 = 3.5;
/// How far an arrow flies, and how straight ahead (the cosine of the angle) a creature must be for it to be hit.
pub const ARROW_RANGE: f32 = 35.0;
const ARROW_LINE: f32 = 0.95;
/// Distance at which a wolf notices the player, and at which it gives up the chase, yards.
const AGGRO_RANGE: f32 = 14.0;
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
const BITE_PAUSE: f32 = 0.6;
const FLINCH_TIME: f32 = 0.45;
/// Chase speed in yards per second: the run animation's own speed.
const RUN_SPEED: f32 = 6.94;
/// Hostile creatures that have not noticed anyone stroll between places within this many yards of home, at this speed, and stand
/// still for a few seconds (between these two numbers) at each.
const WANDER_RADIUS: f32 = 8.0;
const WALK_SPEED: f32 = 2.2;
const REST: (f32, f32) = (4.0, 10.0);
/// A paw hits the ground every this many yards of running (two per stride of the gallop).
const STEP: f32 = 2.3;
/// Creatures further than this from the player are not animated or drawn, yards.
const DRAW_RANGE: f32 = 150.0;
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
    /// What the corpse holds, rolled at its death, until it is looted away or the creature is back.
    pub loot: Option<Loot>,
    rng: u32,
    /// Yards run since the last footstep.
    stride: f32,
    /// Cries made since the last `Creatures::update` took them.
    sounds: Vec<Cry>,
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
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
        }
    }

    /// Put a saved creature back, if it is the same one (the same kind with the same home: the content may have changed since).
    pub fn restore(&mut self, saved: &MobSave) -> bool {
        if saved.kind != self.def.id || flat(saved.home - self.home).length() > 1.0 {
            return false;
        }
        (self.pos, self.yaw, self.health, self.state, self.rng, self.rest, self.pause) = (saved.pos, saved.yaw, saved.health.min(self.def.health), saved.state, saved.rng, saved.rest, saved.pause);
        self.loot = saved.loot.as_ref().map(crate::save::loot_from_save);
        true
    }

    pub fn new(kind: usize, pos: Vec3, yaw: f32) -> Mob {
        let def = &content::get().creatures[kind];
        let mut mob = Mob { kind, def, pos, yaw, home: pos, health: def.health, state: State::Idle, hidden: false, notice: 1.0, pause: 0.0, rest: 0.0, loot: None, rng: pos.x.to_bits() ^ pos.z.to_bits() ^ 0x9E37_79B9, stride: 0.0, sounds: Vec::new() };
        mob.rest = mob.new_rest();
        mob
    }

    /// A number from 0 to 1.
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn new_rest(&mut self) -> f32 {
        REST.0 + (REST.1 - REST.0) * self.random()
    }

    pub fn is_dead(&self) -> bool {
        matches!(self.state, State::Dead(_))
    }

    /// The player's sword hits for `damage`: it flinches, turns on whoever hit it, or dies.
    pub fn hurt(&mut self, damage: f32) {
        if self.is_dead() {
            return;
        }
        self.health -= damage;
        self.sounds.push(if self.health <= 0.0 { Cry::Death } else { Cry::Wound });
        if self.health <= 0.0 {
            self.loot = self.def.loot.as_deref().and_then(content::drop_table).map(|table| table.roll(&mut self.rng));
        }
        self.state = if self.health <= 0.0 { State::Dead(0.0) } else if matches!(self.state, State::Bite(..)) { self.state } else { State::Flinch(0.0) };
    }

    /// The player's sword swing from `from` facing `yaw` reaches this creature if it is hostile, within `reach` yards
    /// and within 60 degrees of straight ahead (or so close that the angle does not matter): it takes `damage`.
    pub fn struck_by(&mut self, from: Vec3, yaw: f32, reach: f32, damage: f32) -> bool {
        let to = flat(self.pos - from);
        let ahead = to.length() < 0.5 || to.normalize().dot(Vec3::new(yaw.sin(), 0.0, -yaw.cos())) > 0.5;
        let hit = self.def.attackable() && !self.is_dead() && to.length() <= reach && ahead;
        if hit {
            self.hurt(damage);
        }
        hit
    }

    /// How close this creature is to noticing a sneaking player at `target` (feet): 0 beyond its normal range, 1 once it has noticed
    /// (a sneaking player is noticed from `SNEAK_AGGRO` of the range, so the way between is what the reticle's eye shows).
    pub fn alert(&self, target: Vec3) -> f32 {
        match self.state {
            _ if !self.def.hostile || self.is_dead() => 0.0,
            State::Return(_) => 0.0,
            State::Idle | State::Wander(_) => ((1.0 - flat(target - self.pos).length() / AGGRO_RANGE) / (1.0 - SNEAK_AGGRO * self.notice)).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }

    /// Advance `dt` seconds with the player at `target` (feet). Returns the damage the creature's bite deals now.
    /// Move by `delta` unless something solid stands in the way (`blocked(from, to)`): then slide along it, one axis at a time, or stay. Returns how
    /// far it went.
    fn walk(&mut self, delta: Vec3, blocked: &impl Fn(Vec3, Vec3) -> bool) -> f32 {
        for d in [delta, Vec3::new(delta.x, 0.0, 0.0), Vec3::new(0.0, 0.0, delta.z)] {
            if d.length() > 1e-6 && !blocked(self.pos, self.pos + d) {
                self.pos += d;
                return d.length();
            }
        }
        0.0
    }

    /// Advance `dt` seconds with the player at `target` (feet), on ground at `ground` where nothing `blocked` is in the way. Returns the damage the
    /// creature's bite deals now.
    pub fn update(&mut self, dt: f32, target: Vec3, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool) -> Option<f32> {
        self.pause = (self.pause - dt).max(0.0);
        let to = flat(target - self.pos);
        let dist = to.length();
        let mut damage = None;
        let face = |yaw: &mut f32| {
            if dist > 0.01 {
                *yaw = to.x.atan2(-to.z);
            }
        };
        match self.state {
            State::Dead(t) => {
                self.state = if t + dt >= self.def.respawn { (self.pos, self.health, self.loot) = (self.home, self.def.health, None); self.rest = self.new_rest(); State::Idle } else { State::Dead(t + dt) };
            }
            State::Flinch(t) => self.state = if t + dt >= FLINCH_TIME { State::Chase } else { State::Flinch(t + dt) },
            State::Idle | State::Wander(_) => {
                if self.def.hostile && dist < if self.hidden { AGGRO_RANGE * SNEAK_AGGRO * self.notice } else { AGGRO_RANGE } {
                    self.state = State::Chase;
                    self.sounds.push(Cry::Aggro);
                } else if self.def.attackable() {
                    match self.state {
                        State::Wander(goal) => {
                            let left = flat(goal - self.pos);
                            if left.length() < 0.3 {
                                (self.state, self.rest) = (State::Idle, self.new_rest());
                            } else {
                                self.yaw = left.x.atan2(-left.z);
                                let step = self.walk(left.normalize() * (WALK_SPEED * dt).min(left.length()), &blocked);
                                if step == 0.0 {
                                    // Something is in the way: it gives up the walk.
                                    (self.state, self.rest) = (State::Idle, self.new_rest());
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
                            if self.rest <= 0.0 {
                                let (angle, radius) = (self.random() * std::f32::consts::TAU, 3.0 + (WANDER_RADIUS - 3.0) * self.random());
                                self.state = State::Wander(self.home + Vec3::new(angle.sin(), 0.0, angle.cos()) * radius);
                            }
                        }
                    }
                }
            }
            State::Chase => {
                face(&mut self.yaw);
                if dist > LEASH_RANGE || flat(self.home - self.pos).length() > LEASH_RANGE {
                    self.state = State::Return(0.0);
                } else if dist <= BITE_RANGE && self.pause == 0.0 {
                    self.state = State::Bite(0.0, false);
                    self.sounds.push(Cry::Attack);
                } else if dist > BITE_RANGE * 0.8 {
                    let step = self.walk(to / dist * (RUN_SPEED * dt).min(dist), &blocked);
                    self.stride += step;
                    if self.stride >= STEP {
                        self.stride -= STEP;
                        self.sounds.push(Cry::Footstep);
                    }
                }
            }
            State::Return(t) if t < RETURN_WAIT => self.state = State::Return(t + dt),
            State::Return(_) => {
                let left = flat(self.home - self.pos);
                if left.length() < 0.3 {
                    (self.state, self.health, self.rest) = (State::Idle, self.def.health, self.new_rest());
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
                face(&mut self.yaw);
                let t = t + dt;
                let landed = hit || t >= BITE_HIT;
                if landed && !hit && dist <= BITE_REACH {
                    damage = Some(self.def.bite_damage);
                    self.sounds.push(Cry::Bite);
                }
                if t >= BITE_TIME {
                    self.pause = BITE_PAUSE;
                    self.state = State::Chase;
                } else {
                    self.state = State::Bite(t, landed);
                }
            }
        }
        if let Some(y) = ground(self.pos.x, self.pos.z, self.pos.y) {
            self.pos.y = y;
        }
        damage
    }

    /// The animation the creature plays.
    fn animation(&self) -> u16 {
        match self.state {
            State::Idle => STAND,
            State::Return(t) if t < RETURN_WAIT => STAND,
            State::Return(_) => RUN,
            State::Wander(_) => WALK,
            State::Chase => RUN,
            State::Bite(..) => BITE,
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

/// A person on screen: a character of some race and sex dressed in his outfit, standing.
pub(crate) struct Person {
    character: Character,
    /// The body, dressed, and what hangs on it.
    worn: Worn,
    /// Whether it was swinging last frame, to start the swing once.
    swinging: bool,
    /// How tall the model is, yards (a tauren is more than a gnome).
    height: f32,
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
        Ok(Person { character, worn, swinging: false, height })
    }

    /// Stand, run after the player, swing the weapon for a bite, or fall and lie (the model's own death animation) when dead.
    #[allow(clippy::too_many_arguments)]
    fn pose(&mut self, dt: f32, pos: Vec3, yaw: f32, state: State, forward: Vec3, viewer: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        let swinging = matches!(state, State::Bite(..));
        if swinging && !self.swinging {
            self.character.attack(BITE_TIME);
        }
        self.swinging = swinging;
        let speed = match state {
            State::Chase => RUN_SPEED,
            State::Return(t) if t >= RETURN_WAIT => RUN_SPEED,
            State::Wander(_) => WALK_SPEED,
            _ => 0.0,
        };
        match state {
            State::Dead(seconds) => self.character.lie(seconds * 1000.0),
            _ => self.character.update(dt, &Motion { speed, forward_speed: speed, ..Motion::default() }, true),
        }
        renderer.update_entity_vertices(gpu, self.worn.body, &self.character.vertices);
        let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
        self.worn.place(renderer, &self.character, pos, orient, false, false);
        self.worn.animate(dt, forward, viewer, renderer, gpu, &self.character, pos, orient, false, false);
    }

    /// Standing, turned north, for the editor's picture; returns the entities to draw.
    pub(crate) fn stand(&mut self, dt: f32, pos: Vec3, forward: Vec3, renderer: &mut Renderer, gpu: &Gpu) -> Vec<usize> {
        self.pose(dt, pos, 0.0, State::Idle, forward, pos - forward * 5.0, renderer, gpu);
        self.worn.entities()
    }

    /// Standing and facing `yaw` (radians, 0 north, clockwise), for the editor's ghost; returns the entities to draw.
    pub(crate) fn stand_facing(&mut self, dt: f32, pos: Vec3, yaw: f32, renderer: &mut Renderer, gpu: &Gpu) -> Vec<usize> {
        let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        self.pose(dt, pos, yaw, State::Idle, forward, pos - forward * 5.0, renderer, gpu);
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
    },
    Person(Person),
}

/// A quest mark's model, shared by every creature that shows it.
struct MarkerRig {
    m2: M2,
    template: Arc<ModelCpu>,
}

struct Creature {
    mob: Mob,
    visual: Visual,
    /// The entities of its three quest marks (those creatures that can be talked to have them), and the one shown.
    marks: Option<[usize; 3]>,
    want: Option<Marker>,
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

pub struct Creatures {
    /// By kind (index into the content's creatures): the model of every kind of beast in the world.
    rigs: Vec<Option<BeastRig>>,
    creatures: Vec<Creature>,
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
}

impl Creatures {
    /// The creatures at `spawns`: (kind, feet position in render space, heading in radians).
    pub fn load(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, spawns: &[(usize, Vec3, f32)]) -> Result<Creatures> {
        let defs = &content::get().creatures;
        let mut rigs: Vec<Option<BeastRig>> = defs.iter().map(|_| None).collect();
        let mut rng = Rng(0x5EED_1234);
        let mut creatures = Vec::new();
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
        for (n, &(kind, pos, yaw)) in spawns.iter().enumerate() {
            let def = &defs[kind];
            let visual = match &def.body {
                Body::Beast { model, skins, scale } => {
                    if rigs[kind].is_none() {
                        let m2 = M2::parse(&mount.read(model)?).with_context(|| format!("{} model", def.name))?;
                        let skins: Vec<(u32, &str)> = skins.iter().map(|(slot, file)| (*slot, file.as_str())).collect();
                        let template = cache.creature(mount, model, &skins)?;
                        let idle_base = m2.sequences.iter().position(|s| s.id == STAND).unwrap_or(0);
                        rigs[kind] = Some(BeastRig { m2, template, idle_base, scale: *scale });
                    }
                    let rig = rigs[kind].as_ref().unwrap();
                    Visual::Beast { entity: renderer.add_entity(gpu, &rig.template), anim: Channel::new(&rig.m2, STAND, &mut rng), last_seq: usize::MAX }
                }
                // A look that does not exist for the race (a face, a hair style) is shown as the default of the race instead.
                Body::Humanoid { outfit, look } => Visual::Person(match Person::load(outfit, look, mount, renderer, gpu, &format!("npc/{n}")) {
                    Ok(person) => person,
                    Err(e) => {
                        log::warn!("{}: {e:#}; showing the default look", def.name);
                        Person::load(outfit, &Appearance { race: look.race, sex: look.sex, skin: 0, face: 0, hair_style: 0, hair_color: 0, beard: 0, baked: 0 }, mount, renderer, gpu, &format!("npc/{n}"))?
                    }
                }),
            };
            // Whoever can be talked to may have a quest mark over its head: one entity for each of the three marks.
            let marks = (def.gossip.is_some() && !markers.is_empty()).then(|| [0, 1, 2].map(|m| renderer.add_entity(gpu, &markers[m].template)));
            creatures.push(Creature { mob: Mob::new(kind, pos, yaw), visual, marks, want: None });
        }
        Ok(Creatures { rigs, creatures, markers, viewer: Vec3::ZERO, rng, clock_ms: 0.0, vertices: Vec::new(), deaths: Vec::new(), target: None, wanted: Vec::new() })
    }

    /// Where the creatures of the given kinds are now (render space): the living ones, else the corpses (which wait to come back).
    pub fn positions(&self, kinds: &[usize]) -> Vec<Vec3> {
        let of_kind = || self.creatures.iter().map(|c| &c.mob).filter(|m| kinds.contains(&m.kind));
        let alive: Vec<Vec3> = of_kind().filter(|m| !m.is_dead()).map(|m| m.pos).collect();
        if alive.is_empty() { of_kind().map(|m| m.pos).collect() } else { alive }
    }

    /// Every creature as it is, in spawn order.
    pub fn snapshot(&self) -> Vec<MobSave> {
        self.creatures.iter().map(|c| c.mob.snapshot()).collect()
    }

    /// Put saved creatures back (the n-th saved is the n-th spawned); those that do not match the spawn list any more start afresh.
    pub fn restore(&mut self, saved: &[MobSave]) {
        let mut back = 0;
        for (c, s) in self.creatures.iter_mut().zip(saved) {
            back += c.mob.restore(s) as usize;
        }
        log::info!("{back} of {} creatures back as saved", self.creatures.len());
    }

    /// Advance every creature and pose it. Returns the damage the bites deal to the player this frame, and the
    /// sounds the creatures made with the place they came from.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, player: Vec3, sneaking: Option<f32>, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool, renderer: &mut Renderer, gpu: &Gpu) -> (Vec<f32>, Vec<(Sfx, Vec3)>) {
        self.clock_ms += dt * 1000.0;
        let (mut bites, mut cries) = (Vec::new(), Vec::new());
        for c in &mut self.creatures {
            let mouth = c.mob.pos + Vec3::Y * 0.8;
            let kind = c.mob.kind;
            cries.extend(c.mob.sounds.drain(..).map(|cry| (Sfx::Cry(cry, kind), mouth)));
            if flat(c.mob.pos - player).length() > DRAW_RANGE {
                // Out of reach nothing is updated: one that was after the player (as the save left it) gives up, or the player would stay in combat.
                if matches!(c.mob.state, State::Chase | State::Bite(..) | State::Flinch(_)) {
                    c.mob.state = State::Return(0.0);
                }
                match &c.visual {
                    Visual::Beast { entity, .. } => renderer.set_entity(*entity, c.mob.pos, Mat4::IDENTITY, false),
                    Visual::Person(p) => p.hide(c.mob.pos, renderer),
                }
                for e in c.marks.into_iter().flatten() {
                    renderer.set_entity(e, c.mob.pos, Mat4::IDENTITY, false);
                }
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
            (c.mob.hidden, c.mob.notice) = (sneaking.is_some(), sneaking.unwrap_or(1.0));
            bites.extend(c.mob.update(dt, player, &ground, &blocked));
            match &mut c.visual {
                Visual::Beast { entity, anim, last_seq } => {
                    let rig = self.rigs[kind].as_ref().expect("a beast has its rig");
                    anim.request(&rig.m2, c.mob.animation(), true, &mut self.rng);
                    anim.advance(&rig.m2, dt * 1000.0, 1.0, true, &mut self.rng);
                    // An idle that turns into a fidget (a look around, a shake) comes with a growl.
                    if c.mob.state == State::Idle && anim.seq() != *last_seq && anim.seq() != rig.idle_base && *last_seq != usize::MAX {
                        cries.push((Sfx::Cry(Cry::Fidget, kind), mouth));
                    }
                    *last_seq = anim.seq();
                    let bones = anim::bone_matrices_with(&rig.m2, anim.mix(&rig.m2), self.clock_ms, &[], &[]);
                    anim::skin(&rig.m2, &bones, &rig.template.vertices, &mut self.vertices);
                    renderer.update_entity_vertices(gpu, *entity, &self.vertices);
                    let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw) * Mat4::from_scale(Vec3::splat(rig.scale));
                    renderer.set_entity(*entity, c.mob.pos, orient, true);
                }
                Visual::Person(person) => person.pose(dt, c.mob.pos, c.mob.yaw, c.mob.state, (c.mob.pos + Vec3::Y - self.viewer).normalize_or_zero(), self.viewer, renderer, gpu),
            }
        }
        (bites, cries)
    }

    /// The player's sword swing: every hostile creature it reaches (see `Mob::struck_by`) takes `damage`. Returns where
    /// above each the damage should show.
    pub fn strike(&mut self, from: Vec3, yaw: f32, damage: f32, unaware_bonus: f32) -> Vec<(Vec3, f32, bool)> {
        let (mut hits, mut ids) = (Vec::new(), Vec::new());
        for (i, c) in self.creatures.iter_mut().enumerate() {
            // A creature that has not noticed the player takes the blow harder (`unaware_bonus`, 1 for none).
            let sneak = matches!(c.mob.state, State::Idle | State::Wander(_)) && unaware_bonus > 1.0;
            let dealt = if sneak { damage * unaware_bonus } else { damage };
            if c.mob.struck_by(from, yaw, SWORD_REACH, dealt) {
                hits.push((c.mob.pos + Vec3::Y * 0.9, dealt, sneak));
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

    /// What the player's arrow from `from` facing `yaw` would hit: the creature that is selected if it is in line and within `range` yards, else the
    /// nearest hostile one in line. Returns it and where it stands.
    pub fn aim(&self, from: Vec3, yaw: f32, range: f32) -> Option<(usize, Vec3)> {
        let ahead = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        let in_line = |c: &Creature| {
            let to = flat(c.mob.pos - from);
            c.mob.def.attackable() && !c.mob.is_dead() && to.length() <= range && to.normalize_or_zero().dot(ahead) > ARROW_LINE
        };
        let near = |c: &Creature| flat(c.mob.pos - from).length();
        let hit = self.target.filter(|&t| self.creatures.get(t).is_some_and(in_line)).or_else(|| {
            (0..self.creatures.len()).filter(|&i| in_line(&self.creatures[i])).min_by(|&a, &b| near(&self.creatures[a]).total_cmp(&near(&self.creatures[b])))
        })?;
        Some((hit, self.creatures[hit].mob.pos))
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

    /// A spell hurts every hostile creature within `radius` yards of `centre`, each for what `roll` gives. Returns where above
    /// each the damage shows, and how much it was.
    pub fn spell_area(&mut self, centre: Vec3, radius: f32, mut roll: impl FnMut() -> f32) -> Vec<(Vec3, f32)> {
        let near: Vec<usize> = (0..self.creatures.len()).filter(|&i| self.enemy_at(i).is_some_and(|p| flat(p - centre).length() <= radius)).collect();
        near.into_iter().filter_map(|i| {
            let damage = roll();
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
        Some(TargetView { id: i, name: m.def.name.clone(), level: m.def.level, fraction: (m.health / m.def.health).clamp(0.0, 1.0), dead: m.is_dead(), hostile: m.def.hostile, neutral: m.def.neutral })
    }

    /// A camera looking at the head of a creature and the entities to draw, for the portrait of the target frame.
    pub fn portrait_shot(&self, creature: usize) -> Option<(Camera, Vec<usize>)> {
        let c = self.creatures.get(creature)?;
        let (distance, height, entities) = match (&c.visual, &c.mob.def.body) {
            (Visual::Person(p), _) => (1.25 * p.tall(), 1.84 * p.tall(), p.worn.entities()),
            (Visual::Beast { entity, .. }, Body::Beast { scale, .. }) => (1.9 * scale, 0.85 * scale, vec![*entity]),
            (Visual::Beast { entity, .. }, _) => (1.9, 0.85, vec![*entity]),
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
        self.creatures.iter().any(|c| matches!(c.mob.state, State::Chase | State::Bite(..) | State::Flinch(_)))
    }

    /// Who is after the player, for the log: the kind, the state and how far from the player.
    pub fn engaged_by(&self, player: Vec3) -> Vec<String> {
        self.creatures.iter().filter(|c| matches!(c.mob.state, State::Chase | State::Bite(..) | State::Flinch(_))).map(|c| format!("{} {:?} at {:.1} yards (home {:.1})", c.mob.def.name, c.mob.state, flat(player - c.mob.pos).length(), flat(player - c.mob.home).length())).collect()
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
    pub fn alert(&self, feet: Vec3) -> f32 {
        self.creatures.iter().map(|c| c.mob.alert(feet)).fold(0.0, f32::max)
    }

    /// The names floating over the living creatures near the camera: red for hostile ones, yellow for neutral ones, green for friendly ones.
    pub fn labels(&self, camera: &Camera, size: (u32, u32)) -> Vec<Label> {
        self.creatures
            .iter()
            .filter(|c| !c.mob.is_dead() && (c.mob.pos - camera.pos).length() <= NAME_RANGE)
            .filter_map(|c| {
                let height = if let Visual::Person(p) = &c.visual { NAME_HEIGHT.1 * p.tall() } else { NAME_HEIGHT.0 };
                let (x, y) = Self::on_screen(camera, size, c.mob.pos + Vec3::Y * height)?;
                let color = if c.mob.def.hostile { [1.0, 0.1, 0.1, 1.0] } else if c.mob.def.neutral { [1.0, 0.9, 0.1, 1.0] } else { [0.1, 1.0, 0.1, 1.0] };
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
        assert!(farthest > 2.0 && farthest <= WANDER_RADIUS + 0.5, "strolled {farthest} yards from home");
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
        let near_but_hidden = Vec3::new(AGGRO_RANGE * SNEAK_AGGRO + 2.0, 0.0, 0.0);
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.hidden = true;
        assert!(wolf.alert(Vec3::new(AGGRO_RANGE + 1.0, 0.0, 0.0)) == 0.0 && wolf.alert(near_but_hidden) > 0.0 && wolf.alert(near_but_hidden) < 1.0, "the eye opens as it comes close");
        assert!(run(&mut wolf, 1.0, near_but_hidden).is_empty() && wolf.state == State::Idle, "not noticed while sneaking");
        wolf.hidden = false;
        run(&mut wolf, 0.1, near_but_hidden);
        assert_eq!(wolf.state, State::Chase, "noticed once he stands up");
        // Sneaking right up to it works only so far.
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.hidden = true;
        run(&mut wolf, 0.1, Vec3::new(AGGRO_RANGE * SNEAK_AGGRO - 1.0, 0.0, 0.0));
        assert_eq!(wolf.state, State::Chase);
    }

    #[test]
    fn a_wolf_follows_the_ground() {
        let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
        wolf.update(0.016, Vec3::new(100.0, 0.0, 0.0), |x, _, _| Some(x * 0.5 + 2.0), open);
        assert_eq!(wolf.pos.y, 2.0);
    }
}
