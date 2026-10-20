//! The weather of the original client (`WorldClient/MapWeather.cpp`): rain, snow and sandstorms. The server told the original which weather and
//! how strong; offline, `Weather` chooses by the zone's climate now and then. What it makes is the original's: every particle is born with a
//! place, a velocity and a birth and a death time, and the video card works out where it is (`weathergpu.rs`, `weather.wgsl`: the originals'
//! vertex programs `rain.bls`, `snowpoint.bls`, `sand.bls` and `patter.bls`). The numbers below are WoW.exe's (FUN_00674c50 for rain,
//! FUN_00677750 snow, FUN_006793f0 sand, FUN_0067a990 the mist, FUN_0067baf0 and FUN_0067bc70 the changes of strength).
//!
//! Positions are in render space (x east, y up, z south); the formulas of the original are written in its world space (x north, y west, z up),
//! and `to_render` converts.

use crate::heavens::to_render;
use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::f32::consts::TAU;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Rain,
    Snow,
    Sand,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Kind> {
        match name.to_ascii_lowercase().as_str() {
            "rain" => Some(Kind::Rain),
            "snow" => Some(Kind::Snow),
            "sand" | "sandstorm" => Some(Kind::Sand),
            _ => None,
        }
    }

    /// The box round the camera the particles are made in: its side and height in yards (the constructors FUN_00674620, FUN_00677420, FUN_00679140).
    fn volume(self) -> (f32, f32) {
        match self {
            Kind::Rain => (130.0, 75.0),
            Kind::Snow => (90.0, 60.0),
            Kind::Sand => (40.0, 25.0),
        }
    }

    /// Particles made per second at full density: the density setters' 0.66 times 35000, 14000 and 32000 (FUN_006749e0, FUN_006776c0, FUN_00679380).
    fn rate(self) -> f32 {
        0.66 * match self {
            Kind::Rain => 35000.0,
            Kind::Snow => 14000.0,
            Kind::Sand => 32000.0,
        }
    }

    /// The most puffs of mist at full density (0.66 times 38, 48 and 64), and how fast they drift: base and spread (the constructors' arguments).
    fn mist(self) -> (f32, (f32, f32)) {
        match self {
            Kind::Rain => (0.66 * 38.0, (5.0, 1.2)),
            Kind::Snow => (0.66 * 48.0, (9.0, 3.0)),
            Kind::Sand => (0.66 * 64.0, (15.0, 4.5)),
        }
    }
}

/// One drop, flake or grain: it is at `pos` at time `t0`, moves with `vel` and is gone at `t1`. Splashes use the same shape (their velocity is zero).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Particle {
    pub pos: [f32; 3],
    pub t0: f32,
    pub vel: [f32; 3],
    pub t1: f32,
}

/// A puff of mist: a big picture of fog that drifts by.
#[derive(Clone, Copy, Debug)]
pub struct Puff {
    pub pos: Vec3,
    pub vel: Vec3,
    pub born: f32,
    pub life: f32,
    pub spin: f32,
}

/// The side of a puff of mist in yards.
pub const PUFF_SIZE: f32 = 12.0;

/// Where the camera is and how it moves.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub eye: Vec3,
    /// Yards per second.
    pub velocity: Vec3,
    /// The way the camera looks, horizontally (render space, need not be normalized).
    pub forward: Vec3,
    pub indoors: bool,
}

/// What the world tells the weather: where the ground is and where things stand that keep the rain off.
pub trait Terrain {
    fn ground(&self, x: f32, z: f32) -> Option<f32>;
    /// The height of the highest terrain or object under `(x, z)` that lies below `from_y`.
    fn solid_below(&self, x: f32, z: f32, from_y: f32) -> Option<f32>;
}

/// The heights at which things stop what falls, by cell of the ground, found when first asked for.
struct Roofs {
    cells: HashMap<(i32, i32), f32>,
    /// The height the cells were found from (they are found again when the camera is far from it).
    top: f32,
}

const CELL: f32 = 1.5;

impl Roofs {
    fn at(&mut self, terrain: &dyn Terrain, x: f32, z: f32) -> f32 {
        let key = ((x / CELL).floor() as i32, (z / CELL).floor() as i32);
        let top = self.top;
        *self.cells.entry(key).or_insert_with(|| {
            let (cx, cz) = ((key.0 as f32 + 0.5) * CELL, (key.1 as f32 + 0.5) * CELL);
            let ground = terrain.ground(cx, cz).unwrap_or(f32::NEG_INFINITY);
            terrain.solid_below(cx, cz, top).unwrap_or(f32::NEG_INFINITY).max(ground)
        })
    }
}

struct Rng(u64);

impl Rng {
    /// 0 to 1.
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    /// -0.5 to 0.5.
    fn centred(&mut self) -> f32 {
        self.unit() - 0.5
    }
}

/// Interpolation clamped to 0..1 (FUN_00674ba0).
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// How long the weather takes to change by a whole unit of strength, in seconds (FUN_0067bc70).
const RAMP_SECONDS: f32 = 10.0;

pub struct Weather {
    /// The share of the weather that falls (the weather intensity of the video options).
    pub detail: f32,
    /// The weather that shows (or is fading out), and how strong it is now, 0 to 1.
    kind: Option<Kind>,
    intensity: f32,
    /// What it is changing to.
    next: Option<Kind>,
    goal: f32,
    /// Seconds on the weather's clock, which the particles' times are in.
    pub clock: f32,
    pub particles: Vec<Particle>,
    pub splashes: Vec<Particle>,
    pub puffs: Vec<Puff>,
    owed: f32,
    mist_owed: f32,
    rng: Rng,
    roofs: Roofs,
    /// The weather comes and goes by itself while this is on (`--weather` fixes it).
    pub auto: bool,
    /// Seconds until `auto` changes the weather.
    timer: f32,
}

impl Weather {
    pub fn new(seed: u64) -> Self {
        Weather {
            detail: 1.0,
            kind: None,
            intensity: 0.0,
            next: None,
            goal: 0.0,
            clock: 0.0,
            particles: Vec::new(),
            splashes: Vec::new(),
            puffs: Vec::new(),
            owed: 0.0,
            mist_owed: 0.0,
            rng: Rng(seed | 1),
            roofs: Roofs { cells: HashMap::new(), top: 0.0 },
            auto: true,
            timer: 60.0,
        }
    }

    pub fn kind(&self) -> Option<Kind> {
        self.kind
    }

    #[cfg(test)]
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// How stormy the sky is: the sky is overcast once the strength is a quarter (FUN_0067bc70 gives the sky the strength to a quarter).
    pub fn storm(&self) -> f32 {
        (self.intensity.min(0.25) * 4.0).clamp(0.0, 1.0)
    }

    /// How much falls: nothing below a quarter, the full amount at full strength.
    pub fn density(&self) -> f32 {
        ((self.intensity - 0.25) * 4.0 / 3.0).clamp(0.0, 1.0) * self.detail
    }

    /// Changes the weather to `kind` at strength `strength`: another kind first fades the one that shows out. `now` skips the fading.
    pub fn set(&mut self, kind: Option<Kind>, strength: f32, now: bool) {
        let strength = if kind.is_some() { strength.clamp(0.0, 1.0) } else { 0.0 };
        self.next = kind;
        self.goal = strength;
        if now {
            self.kind = kind;
            self.intensity = strength;
        }
    }

    /// Whether anything shows or falls.
    #[cfg(test)]
    pub fn active(&self) -> bool {
        self.kind.is_some() || !self.particles.is_empty() || !self.puffs.is_empty() || !self.splashes.is_empty()
    }

    /// The sound that goes with the weather now: its file and how loud, by the strength (thirds: light, medium, heavy).
    pub fn sound(&self) -> Option<(&'static str, f32)> {
        let kind = self.kind?;
        if self.intensity <= 0.05 {
            return None;
        }
        let heavy = (self.intensity * 3.0) as usize;
        let file = match (kind, heavy.min(2)) {
            (Kind::Rain, 0) => r"Sound\Ambience\Weather\RainLightLoop.wav",
            (Kind::Rain, 1) => r"Sound\Ambience\Weather\RainMediumLoop.wav",
            (Kind::Rain, _) => r"Sound\Ambience\Weather\RainHeavyLoop.wav",
            (Kind::Snow, 0) => r"Sound\Ambience\Weather\SnowLight.wav",
            (Kind::Snow, 1) => r"Sound\Ambience\Weather\SnowMedium.wav",
            (Kind::Snow, _) => r"Sound\Ambience\Weather\SnowHeavy.wav",
            (Kind::Sand, 0) => r"Sound\Ambience\Weather\SandStormLight.wav",
            (Kind::Sand, 1) => r"Sound\Ambience\Weather\SandStormMedium.wav",
            (Kind::Sand, _) => r"Sound\Ambience\Weather\SandStormHeavy.wav",
        };
        Some((file, self.intensity.min(1.0)))
    }

    /// Moves the weather on by `dt` seconds: the change of strength, the choosing of new weather, the particles' making and passing.
    /// `climate` is what the zone allows (`climate_of`).
    pub fn update(&mut self, dt: f32, motion: &Motion, terrain: &dyn Terrain, climate: Option<Kind>) {
        self.clock += dt;
        if self.auto {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.choose(climate);
            }
        }
        // The strength moves towards the goal, a whole unit in ten seconds. Another kind waits until this one is gone.
        let goal = if self.next == self.kind { self.goal } else { 0.0 };
        let step = dt / RAMP_SECONDS;
        self.intensity = if self.intensity < goal { (self.intensity + step).min(goal) } else { (self.intensity - step).max(goal) };
        if self.next != self.kind && self.intensity <= 0.0 {
            self.kind = self.next;
        }

        let now = self.clock;
        self.particles.retain(|p| p.t1 + TAIL >= now);
        self.splashes.retain(|p| p.t1 >= now);
        self.puffs.retain(|p| p.born + p.life >= now);

        let Some(kind) = self.kind else { return };
        if (self.roofs.top - (motion.eye.y + 40.0)).abs() > 12.0 {
            self.roofs.cells.clear();
            self.roofs.top = motion.eye.y + 40.0;
        }
        if self.roofs.cells.len() > 20_000 {
            let eye = motion.eye;
            self.roofs.cells.retain(|&(cx, cz), _| ((cx as f32 * CELL - eye.x).abs() < 110.0) && ((cz as f32 * CELL - eye.z).abs() < 110.0));
        }
        let density = self.density();
        self.owed += kind.rate() * density * dt.min(0.1);
        let count = (self.owed as usize).min(MOST_PER_FRAME);
        self.owed -= self.owed.floor();
        for i in 0..count {
            let born = self.clock - dt + dt * (i as f32 + self.rng.unit()) / count as f32;
            self.spawn(kind, density, born, motion, terrain);
        }
        if !motion.indoors {
            let (most, speed) = kind.mist();
            let wanted = ((density - 0.5).max(0.0) * 2.0 * most) as usize;
            // The puffs live 2.7 seconds: that many a second keep the number up.
            self.mist_owed += wanted as f32 / 2.7 * dt;
            while self.mist_owed >= 1.0 && self.puffs.len() < wanted {
                self.mist_owed -= 1.0;
                self.spawn_puff(speed, motion, terrain);
            }
        }
    }

    /// New weather, by what the zone allows: it clears for a while, then something comes up.
    fn choose(&mut self, climate: Option<Kind>) {
        if self.next.is_some() {
            self.set(None, 0.0, false);
            self.timer = 240.0 + self.rng.unit() * 600.0;
        } else if let Some(kind) = climate.filter(|_| self.rng.unit() < 0.7) {
            let strength = 0.35 + self.rng.unit() * 0.65;
            self.set(Some(kind), strength, false);
            self.timer = 90.0 + self.rng.unit() * 240.0;
        } else {
            self.timer = 120.0 + self.rng.unit() * 300.0;
        }
    }

    /// Makes one particle born at `born` (FUN_00674c50, FUN_00677750 and FUN_006793f0).
    fn spawn(&mut self, kind: Kind, d: f32, born: f32, m: &Motion, terrain: &dyn Terrain) {
        let (w, h) = kind.volume();
        let r = &mut self.rng;
        // The original's frame: x north, y west, z up; the camera looks along `facing`.
        let forward = Vec3::new(-m.forward.z, -m.forward.x, 0.0).normalize_or_zero();
        let facing = forward.y.atan2(forward.x);
        if kind == Kind::Sand {
            // The grains start ahead of the camera and fly at it, in the camera's own frame.
            let theta = r.centred() * 0.349066 - 1.57;
            let s = r.centred() * 0.666667 + 18.6667;
            let across = |v: Vec3| forward * v.x + Vec3::new(-forward.y, forward.x, 0.0) * v.y;
            let vel = across(Vec3::new(theta.sin() * s, theta.cos() * s, 0.0)) + Vec3::Z * (r.centred() * 0.166667 + 0.833333);
            let pos = across(Vec3::new((r.unit() * 0.15 + 0.85) * w, r.centred() * w, 0.0)) + Vec3::Z * (r.centred() * h);
            let look = 3.2 + r.centred() * 0.3;
            return self.finish(kind, pos, vel, born, m, terrain, look);
        }
        // Rain falls at 28 yards a second and more, nearly straight; snow drifts down at 2 to 6, and the stronger it is the more
        // of it goes the same way as the wind.
        let (theta, speed, vz) = match kind {
            Kind::Rain => (r.centred() * (d * 0.20944 + 0.0523599) - 1.57, (r.centred() * 2.0 + 9.49) * d + 0.01, r.unit() * d * -2.0 - d * 4.0 - 28.0),
            _ => (r.centred() * (TAU - d * 5.93412) - 1.57, (r.centred() + 5.985) * d + 0.015, r.unit() * d * -1.0 - d * 3.5 - 2.0),
        };
        let vel = Vec3::new(theta.sin() * speed, theta.cos() * speed, vz);
        // It passes through the middle plane of the box after half the box's height of falling: it starts that far up, upwind.
        let fall = (h / 2.0) / -vz;
        let mut start = Vec3::new(r.centred() * w, r.centred() * w, 0.0) - vel * fall;
        // Moving fast, the weather comes at you: the field of particles leans towards the way you go.
        // shortcut: the sign of the lean (towards the way you go) is chosen by what is physical, not read from FUN_007bdd60; flip it if the rain
        // looks to run away from you.
        let speed_now = Vec3::new(m.velocity.x, 0.0, m.velocity.z).length();
        let lean = lerp(0.0, 1.13446, speed_now * 0.0555556);
        let axis = Vec3::new(facing.sin(), -facing.cos(), 0.0);
        if lean > 1e-4 && axis.length() > 0.5 {
            start = Quat::from_axis_angle(axis.normalize(), -lean) * start;
        }
        self.finish(kind, start, vel, born, m, terrain, fall * 2.5)
    }

    /// Places the particle round the camera (and where it will be in 1.75 seconds), finds when it hits something and stores it.
    fn finish(&mut self, kind: Kind, offset_w: Vec3, vel_w: Vec3, born: f32, m: &Motion, terrain: &dyn Terrain, reach: f32) {
        let pos = m.eye + to_render(offset_w) + m.velocity * 1.75;
        let vel = to_render(vel_w);
        if terrain.ground(pos.x, pos.z).is_none_or(|g| pos.y <= g) {
            return;
        }
        if self.roofs.at(terrain, pos.x, pos.z) >= pos.y {
            return;
        }
        let lifetime = self.trace(pos, vel, reach, terrain);
        if lifetime <= 0.0 || lifetime > 60.0 {
            return;
        }
        // Sand lasts a fraction of a second (the original's own ratio of the distance to the hit to the look-ahead).
        let lifetime = if kind == Kind::Sand { (lifetime / reach).min(1.0) } else { lifetime };
        let particle = Particle { pos: pos.to_array(), t0: born, vel: vel.to_array(), t1: born + lifetime };
        self.particles.push(particle);
        // A drop that lands on the ground and finds the camera standing still splashes (FUN_006754a0).
        let speed2 = m.velocity.length_squared();
        if kind == Kind::Rain && !m.indoors && speed2 <= 2.0 {
            let hit = pos + vel * lifetime;
            if terrain.ground(hit.x, hit.z).is_some_and(|g| (hit.y - g).abs() < 0.6) {
                self.splashes.push(Particle { pos: hit.to_array(), t0: born + lifetime, vel: [0.0; 3], t1: born + lifetime + 0.25 });
            }
        }
    }

    /// Seconds until `pos` moving with `vel` is stopped by the ground or a roof, at most `limit`.
    fn trace(&mut self, pos: Vec3, vel: Vec3, limit: f32, terrain: &dyn Terrain) -> f32 {
        let length = vel.length() * limit;
        let steps = ((length / (CELL * 0.75)).ceil() as usize).clamp(1, 80);
        let mut prev = 0.0;
        for i in 1..=steps {
            let t = limit * i as f32 / steps as f32;
            let p = pos + vel * t;
            let roof = self.roofs.at(terrain, p.x, p.z);
            if p.y <= roof {
                // Between the last step and this one: where the line crosses the roof.
                let before = pos + vel * prev;
                let above = before.y - self.roofs.at(terrain, before.x, before.z);
                let below = roof - p.y;
                return prev + (t - prev) * above.max(0.0) / (above.max(0.0) + below).max(1e-4);
            }
            prev = t;
        }
        limit
    }

    fn spawn_puff(&mut self, speed: (f32, f32), m: &Motion, terrain: &dyn Terrain) {
        let r = &mut self.rng;
        let forward = Vec3::new(-m.forward.z, -m.forward.x, 0.0).normalize_or_zero();
        let across = |v: Vec3| forward * v.x + Vec3::new(-forward.y, forward.x, 0.0) * v.y;
        let theta = r.centred() * 0.349 - 1.57;
        let s = r.centred() * speed.1 + speed.0;
        let vel_w = across(Vec3::new(theta.sin() * s, theta.cos() * s, 0.0)) + Vec3::Z * (r.centred() * 0.3);
        let local = across(Vec3::new(r.centred() * 44.0, r.centred() * 44.0, 0.0)) + Vec3::Z * (r.centred() * 25.0);
        let vel = to_render(vel_w);
        let mut pos = m.eye - vel * 2.5 + to_render(local);
        let floor = terrain.ground(pos.x, pos.z).unwrap_or(m.eye.y - 2.0);
        pos.y = pos.y.max(floor) + PUFF_SIZE * 0.25;
        self.puffs.push(Puff { pos, vel, born: self.clock, life: 2.7 + r.centred() * 0.3, spin: r.centred() * 3.3333 });
    }
}

/// How long after its death a rain drop's tail still falls (the tail is two yards behind the head); the longest is at the slowest fall.
pub const TAIL: f32 = 0.1;

/// The most particles made in one frame.
const MOST_PER_FRAME: usize = 1200;

/// What weather a zone has, by its name: rain nearly everywhere, snow in the cold ones, sandstorms in the deserts, none in the driest places.
pub fn climate_of(zone: &str) -> Option<Kind> {
    const SNOW: &[&str] = &["Dun Morogh", "Alterac Mountains", "Winterspring", "Coldridge", "Frostmane", "Ironforge", "Wintergrasp"];
    const SAND: &[&str] = &["Tanaris", "Silithus", "Uldum", "Gadgetzan", "Thousand Needles", "Barrens"];
    const NONE: &[&str] = &["Durotar", "Badlands", "Searing Gorge", "Burning Steppes", "Blasted Lands", "Orgrimmar", "Deadwind", "Stormwind City"];
    let has = |names: &[&str]| names.iter().any(|n| zone.contains(n));
    if has(NONE) {
        None
    } else if has(SNOW) {
        Some(Kind::Snow)
    } else if has(&SAND[..4]) {
        Some(Kind::Sand)
    } else {
        Some(Kind::Rain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat world with ground at 0 and a roof of 10 yards over the square within 5 yards of the origin.
    struct Flat;

    impl Terrain for Flat {
        fn ground(&self, _: f32, _: f32) -> Option<f32> {
            Some(0.0)
        }

        fn solid_below(&self, x: f32, z: f32, _: f32) -> Option<f32> {
            Some(if x.abs() < 5.0 && z.abs() < 5.0 { 10.0 } else { 0.0 })
        }
    }

    fn standing() -> Motion {
        Motion { eye: Vec3::new(30.0, 2.0, 30.0), velocity: Vec3::ZERO, forward: Vec3::new(0.0, 0.0, -1.0), indoors: false }
    }

    fn weather(kind: Kind, strength: f32) -> Weather {
        let mut w = Weather::new(7);
        w.auto = false;
        w.set(Some(kind), strength, true);
        w
    }

    fn run(w: &mut Weather, seconds: f32) {
        let m = standing();
        for _ in 0..(seconds * 30.0) as usize {
            w.update(1.0 / 30.0, &m, &Flat, None);
        }
    }

    #[test]
    fn the_sky_is_stormy_from_a_quarter_and_things_fall_above_that() {
        let mut w = Weather::new(1);
        w.set(Some(Kind::Rain), 0.25, true);
        assert_eq!(w.storm(), 1.0);
        assert_eq!(w.density(), 0.0);
        w.set(Some(Kind::Rain), 1.0, true);
        assert_eq!(w.density(), 1.0);
        w.set(Some(Kind::Rain), 0.125, true);
        assert!((w.storm() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_strength_changes_a_whole_unit_in_ten_seconds_and_another_kind_waits_for_the_first_to_go() {
        let mut w = Weather::new(1);
        w.auto = false;
        w.set(Some(Kind::Rain), 1.0, false);
        // The first change from nothing: the kind shows once it is the goal and nothing else shows.
        run(&mut w, 5.0);
        assert_eq!(w.kind(), Some(Kind::Rain));
        assert!((w.intensity() - 0.5).abs() < 0.02);
        w.set(Some(Kind::Snow), 1.0, false);
        run(&mut w, 4.0);
        assert_eq!(w.kind(), Some(Kind::Rain));
        run(&mut w, 2.0);
        assert_eq!(w.kind(), Some(Kind::Snow));
        assert!(w.intensity() < 0.1);
    }

    #[test]
    fn rain_makes_drops_that_fall_and_stop_at_the_ground_or_a_roof() {
        let mut w = weather(Kind::Rain, 1.0);
        run(&mut w, 1.5);
        assert!(w.particles.len() > 5000, "{} drops", w.particles.len());
        for p in &w.particles {
            let (pos, vel) = (Vec3::from(p.pos), Vec3::from(p.vel));
            assert!(vel.y < -25.0, "falls at {}", vel.y);
            // The end of its fall is at or just above the ground or the roof, never far under.
            let end = pos + vel * (p.t1 - p.t0);
            // (The roofs are known by cells of a yard and a half: the edge of one is not exact.)
            let roof = if end.x.abs() < 3.0 && end.z.abs() < 3.0 { 10.0 } else { 0.0 };
            assert!(end.y > roof - 2.0, "ends at {} on a roof at {roof}", end.y);
        }
        // Standing still, drops that land on the ground splash.
        assert!(!w.splashes.is_empty());
    }

    #[test]
    fn nothing_falls_through_a_roof() {
        let mut w = weather(Kind::Rain, 1.0);
        let m = Motion { eye: Vec3::new(0.0, 2.0, 0.0), ..standing() };
        for _ in 0..60 {
            w.update(1.0 / 30.0, &m, &Flat, None);
        }
        // Over the roof the drops end at 10, not at the ground.
        let landed_inside = w.particles.iter().any(|p| {
            let end = Vec3::from(p.pos) + Vec3::from(p.vel) * (p.t1 - p.t0);
            end.x.abs() < 3.0 && end.z.abs() < 3.0 && end.y < 9.0
        });
        assert!(!landed_inside);
    }

    #[test]
    fn snow_drifts_down_slowly_and_sand_flies_at_the_camera() {
        let mut w = weather(Kind::Snow, 1.0);
        run(&mut w, 2.0);
        assert!(!w.particles.is_empty());
        assert!(w.particles.iter().all(|p| p.vel[1] < -1.9 && p.vel[1] > -7.0));
        let mut w = weather(Kind::Sand, 1.0);
        run(&mut w, 1.0);
        assert!(!w.particles.is_empty());
        // The camera looks to -z; the grains come from there towards +z.
        let toward = w.particles.iter().filter(|p| p.vel[2] > 10.0).count();
        assert!(toward * 10 > w.particles.len() * 9, "{toward} of {}", w.particles.len());
        assert!(w.particles.iter().all(|p| p.t1 - p.t0 <= 1.0));
    }

    #[test]
    fn it_stops_when_the_weather_clears_and_the_particles_are_gone() {
        let mut w = weather(Kind::Rain, 1.0);
        run(&mut w, 1.0);
        w.set(None, 0.0, false);
        run(&mut w, 14.0);
        assert_eq!(w.kind(), None);
        assert!(!w.active());
    }

    #[test]
    fn zones_have_their_climate() {
        assert_eq!(climate_of("Elwynn Forest - Northshire Valley"), Some(Kind::Rain));
        assert_eq!(climate_of("Dun Morogh - Coldridge Valley"), Some(Kind::Snow));
        assert_eq!(climate_of("Tanaris"), Some(Kind::Sand));
        assert_eq!(climate_of("Durotar"), None);
    }

    #[test]
    fn the_sound_follows_the_kind_and_the_strength() {
        let w = weather(Kind::Rain, 0.2);
        assert!(w.sound().unwrap().0.ends_with("RainLightLoop.wav"));
        let w = weather(Kind::Rain, 0.9);
        assert!(w.sound().unwrap().0.ends_with("RainHeavyLoop.wav"));
        assert!(weather(Kind::Snow, 0.5).sound().unwrap().0.contains("Snow"));
        assert!(Weather::new(1).sound().is_none());
    }
}
