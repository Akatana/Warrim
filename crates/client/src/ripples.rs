//! The ripples of a character in water, ported from the original client: `FUN_005fa760` makes them while the water is less than twice the
//! character's height deep (a ring of `splash.blp` where it stands, turns or has just stepped in, a V of `wake.blp` where it moves), `FUN_0068f5c0`
//! makes each one a decal on the liquid cells under it, and `FUN_0068fae0` grows and fades it and draws it (additively, flat on the water).
//! The numbers are the original's constants (see docs/RE-FINDINGS.md, "Ripples"). Shortcut: only the local player makes ripples (the original keeps
//! a second pool of 96 for other units).

use crate::models::{BatchCpu, ModelCache, ModelCpu, ModelVertex};
use crate::particles::quad_indices;
use crate::renderer::{Gpu, Renderer};
use glam::{Mat4, Vec3};
use mpq::Mount;
use wowfmt::m2::Blend;

const SPLASH: &str = r"XTextures\splash\splash.blp";
const WAKE: &str = r"XTextures\splash\wake.blp";
/// Ripples alive at once (the original's pool for the local player), and the liquid cells a ripple is laid on at the most.
const POOL: usize = 32;
const CELLS: usize = 12;
/// The share of its life a ripple takes to reach its full brightness (0x810348).
const FADE_IN: f32 = 0.4;
/// Raised off the water so that it does not fight the surface for the depth buffer (the original biases the depth by `footstepBias`).
const LIFT: f32 = 0.03;

/// What the character is doing, which says what ripple it makes (`FUN_005fa760`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Idle,
    Turning,
    Moving,
    /// The water has just reached the knees (or gone from them).
    Entering,
}

/// The character in the water, as the ripples see it.
pub struct Walker {
    pub feet: Vec3,
    /// The surface of the water at the feet.
    pub level: f32,
    /// The height of the character (the hull's, 2.03 yards for a human).
    pub height: f32,
    /// The scale of its model.
    pub scale: f32,
    /// Yards a second.
    pub speed: f32,
    /// The way it moves, a unit vector over the water (x east, z south): the way it faces, turned by the strafing and backing of the keys.
    pub heading: (f32, f32),
}

struct Ripple {
    centre: Vec3,
    heading: (f32, f32),
    radius: f32,
    growth: f32,
    peak: f32,
    alpha: f32,
    /// Alpha gained a second while rising (0 once the peak is passed) and gained (negative) while fading.
    rise: f32,
    fall: f32,
    left: f32,
    /// 0: a ring, 1: a V.
    group: usize,
    cells: Vec<[Vec3; 4]>,
}

pub struct Ripples {
    rng: u32,
    now: f32,
    /// When the next one may be made.
    next: f32,
    pool: Vec<Ripple>,
}

impl Default for Ripples {
    fn default() -> Ripples {
        Ripples { rng: 0x2545_F491, now: 0.0, next: 0.0, pool: Vec::new() }
    }
}

impl Ripples {
    /// A number from 0 up to but not 1.
    fn dice(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `FUN_005fa760`: make the ripple the character calls for now, if its time has come. `cells(centre, half)` says the liquid cells (their corners,
    /// render space) round a point.
    pub fn make(&mut self, w: &Walker, state: State, cells: impl Fn(Vec3, f32) -> Vec<[Vec3; 4]>) {
        let reach = (2.0 * w.height).max(1.0);
        let depth = w.level - w.feet.y;
        if depth >= reach {
            return;
        }
        if state == State::Entering {
            self.next = 0.0;
        }
        if self.next != 0.0 && self.now < self.next {
            return;
        }
        let moving = state == State::Moving;
        // A moving character makes them faster the faster it goes (a walk: every quarter second), and they spread faster.
        let pace = if moving && w.speed > 1e-4 { w.speed.min(20.0) / 2.5 } else { 1.0 };
        let (r0, r1, r2) = (self.dice(), self.dice(), self.dice());
        let mut size = ((w.scale / 3.0) * (1.0 + r0 * 0.2 - 0.1)).clamp(1.0 / 3.0, 5.0 / 3.0);
        let mut life = r1 * 0.1 - 0.05 + 0.65;
        let mut growth = (r2 * 1.5 - 0.75 + 3.75) * 12.0 / 36.0 * pace;
        let mut brightness = 1.0 / 6.0;
        // Deeper than the character is tall the ripples shrink and dim, down to half at twice the height.
        let shrink = if depth > w.height { ((reach - depth) / (reach - w.height)) * 0.5 + 0.5 } else { 1.0 };
        if depth > w.height {
            (brightness, life, size) = (shrink / 6.0, life * shrink, size * shrink);
        }
        if state == State::Idle {
            (brightness, life, size, growth) = (brightness * 0.8, life, size * 0.6, growth * 0.25);
        }
        let heading = if moving {
            w.heading
        } else {
            let a = self.dice() * std::f32::consts::TAU;
            (a.sin(), -a.cos())
        };
        let peak = if brightness < 1.0 / 6.0 { brightness * 6.0 } else { 1.0 };
        let group = moving as usize;
        // The decal lies on the liquid under the whole of its final size.
        let centre = Vec3::new(w.feet.x, w.level, w.feet.z);
        let mut cells = cells(centre, size + growth * life);
        cells.truncate(CELLS);
        let ripple = Ripple { centre, heading, radius: size, growth, peak, alpha: 0.0, rise: peak / (life * FADE_IN), fall: -peak / ((1.0 - FADE_IN) * life), left: life, group, cells };
        if self.pool.len() >= POOL {
            self.pool.remove(0);
        }
        self.pool.push(ripple);
        self.next = self.now + if moving { shrink * (1.0 / pace) * 0.25 } else { 0.4 + self.dice() * 0.05 };
    }

    /// `FUN_0068fae0`'s update: every ripple spreads, brightens to its peak in the first 0.4 of its life and fades out over the rest.
    pub fn update(&mut self, dt: f32) {
        self.now += dt;
        for r in &mut self.pool {
            r.left -= dt;
            r.radius += r.growth * dt;
            let mut rest = dt;
            if r.rise != 0.0 {
                r.alpha += r.rise * dt;
                if r.alpha > r.peak {
                    rest = dt - (r.alpha - r.peak) / r.rise;
                    (r.alpha, r.rise) = (r.peak, 0.0);
                }
            }
            if r.rise == 0.0 {
                r.alpha += rest * r.fall;
            }
        }
        self.pool.retain(|r| r.left > 0.0 && r.alpha > 0.0);
    }

    pub fn is_empty(&self) -> bool {
        self.pool.is_empty()
    }

    /// The quads of the ripples of `group` (0 rings, 1 Vs), their corners relative to `origin`, into `out` (the rest of it is left empty).
    fn quads(&self, group: usize, origin: Vec3, out: &mut [ModelVertex]) {
        out.iter_mut().for_each(|v| v.pos = [0.0; 3]);
        let mut next = 0;
        for r in self.pool.iter().filter(|r| r.group == group) {
            let color = [255, 255, 255, (r.alpha.clamp(0.0, 1.0) * 255.0) as u8];
            let right = (-r.heading.1, r.heading.0);
            for corner in r.cells.iter().flatten().collect::<Vec<_>>().chunks(4) {
                for p in corner {
                    // The picture is laid with its top towards the way the ripple points, centred, as large across as twice its radius.
                    let (dx, dz) = (p.x - r.centre.x, p.z - r.centre.z);
                    let uv = [0.5 + (dx * right.0 + dz * right.1) / (2.0 * r.radius), 0.5 - (dx * r.heading.0 + dz * r.heading.1) / (2.0 * r.radius)];
                    out[next] = ModelVertex { pos: (**p + Vec3::Y * LIFT - origin).to_array(), normal: [0.0, 1.0, 0.0], uv, color };
                    next += 1;
                }
            }
        }
    }
}

/// The two entities that draw the ripples: one for the rings and one for the Vs (two pictures, so two batches).
pub struct RippleDraw {
    entities: [usize; 2],
    vertices: Vec<ModelVertex>,
    /// Whether what is drawn is empty (so it need not be written again).
    clear: bool,
}

impl RippleDraw {
    pub fn new(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu) -> Option<RippleDraw> {
        let quads = POOL * CELLS;
        let mut entities = [0; 2];
        for (group, path) in [SPLASH, WAKE].into_iter().enumerate() {
            let texture = cache.texture(mount, path)?;
            let cpu = ModelCpu {
                key: format!("ripples/{group}"),
                vertices: vec![ModelVertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], uv: [0.0; 2], color: [0; 4] }; quads * 4],
                indices: quad_indices(quads),
                batches: vec![BatchCpu { index_start: 0, index_count: (quads * 6) as u32, texture: Some(texture), wrap: false, blend: Blend::Additive, unlit: true, baked: Default::default(), window: false, glow: [0; 3], refs: Default::default(), particle: true }],
                col_vertices: Vec::new(),
                col_indices: Vec::new(),
                tracks: None,
            };
            entities[group] = renderer.add_entity(gpu, &cpu);
            renderer.set_entity(entities[group], Vec3::ZERO, Mat4::IDENTITY, false);
        }
        Some(RippleDraw { entities, vertices: vec![ModelVertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], uv: [0.0; 2], color: [0; 4] }; quads * 4], clear: true })
    }

    /// Show the ripples, their corners measured from `origin` (somewhere near, so that the numbers stay small).
    pub fn draw(&mut self, ripples: &Ripples, origin: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        if ripples.is_empty() && self.clear {
            return;
        }
        self.clear = ripples.is_empty();
        for (group, &entity) in self.entities.iter().enumerate() {
            ripples.quads(group, origin, &mut self.vertices);
            renderer.update_entity_vertices(gpu, entity, &self.vertices);
            renderer.set_entity(entity, origin, Mat4::IDENTITY, !self.clear);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walker(depth: f32, speed: f32) -> Walker {
        Walker { feet: Vec3::new(10.0, 5.0, 20.0), level: 5.0 + depth, height: 2.0278, scale: 1.0, speed, heading: (0.0, -1.0) }
    }

    /// One cell of water under the point.
    fn cells(c: Vec3, _: f32) -> Vec<[Vec3; 4]> {
        let (x, z, y) = (c.x.floor(), c.z.floor(), c.y);
        vec![[Vec3::new(x, y, z), Vec3::new(x + 4.0, y, z), Vec3::new(x + 4.0, y, z + 4.0), Vec3::new(x, y, z + 4.0)]]
    }

    #[test]
    fn nothing_is_made_where_the_water_is_twice_as_deep_as_one_is_tall() {
        let mut r = Ripples::default();
        r.make(&walker(4.1, 0.0), State::Idle, cells);
        assert!(r.is_empty());
        r.make(&walker(0.5, 0.0), State::Idle, cells);
        assert_eq!(r.pool.len(), 1);
    }

    #[test]
    fn a_standing_one_makes_small_rings_a_walker_makes_vs_that_spread_faster_and_come_quicker() {
        let mut r = Ripples::default();
        r.make(&walker(0.5, 0.0), State::Idle, cells);
        let ring = &r.pool[0];
        // A third of a yard, six tenths of it, a ring (picture 0), bright to 0.8, and alive some six tenths of a second.
        assert!((ring.radius - 0.2).abs() < 0.03 && ring.group == 0 && (ring.peak - 0.8).abs() < 1e-4, "{} {} {}", ring.radius, ring.group, ring.peak);
        assert!((0.59..0.71).contains(&ring.left) && (0.25..0.38).contains(&ring.growth), "{} {}", ring.left, ring.growth);
        // The next waits 0.4 s and a little.
        assert!((0.4..0.45).contains(&r.next));
        let mut w = Ripples::default();
        w.make(&walker(0.5, 2.5), State::Moving, cells);
        let v = &w.pool[0];
        assert!(v.group == 1 && (1.0..=1.5).contains(&v.growth) && (v.radius - 1.0 / 3.0).abs() < 0.04 && v.peak == 1.0, "{} {}", v.growth, v.radius);
        assert!((w.next - 0.25).abs() < 1e-4, "a walker makes one every quarter second, not {}", w.next);
        let mut run = Ripples::default();
        run.make(&walker(0.5, 7.0), State::Moving, cells);
        assert!((run.next - 0.25 * 2.5 / 7.0).abs() < 1e-4 && run.pool[0].growth > 2.7, "{} {}", run.next, run.pool[0].growth);
    }

    #[test]
    fn a_bigger_character_makes_bigger_ripples() {
        let mut small = Ripples::default();
        small.make(&walker(0.5, 2.5), State::Moving, cells);
        let mut big = Ripples::default();
        big.make(&Walker { scale: 2.0, height: 4.0, ..walker(0.5, 2.5) }, State::Moving, cells);
        assert!(big.pool[0].radius > 1.5 * small.pool[0].radius, "{} {}", big.pool[0].radius, small.pool[0].radius);
    }

    #[test]
    fn deeper_than_ones_height_the_ripples_shrink_and_dim_and_stepping_in_makes_one_at_once() {
        let mut r = Ripples::default();
        r.make(&walker(3.0, 2.5), State::Moving, cells);
        let deep = &r.pool[0];
        assert!(deep.peak < 0.8 && deep.peak > 0.5 && deep.left < 0.6, "{} {}", deep.peak, deep.left);
        r.update(0.05);
        r.make(&walker(0.5, 0.0), State::Idle, cells);
        assert_eq!(r.pool.len(), 1, "waits for its time");
        r.make(&walker(0.8, 0.0), State::Entering, cells);
        assert_eq!(r.pool.len(), 2);
        assert!(r.pool[1].group == 0);
    }

    #[test]
    fn a_ripple_brightens_for_two_fifths_of_its_life_and_fades_to_nothing_and_goes() {
        let mut r = Ripples::default();
        r.make(&walker(0.5, 2.5), State::Moving, cells);
        let (life, peak) = (r.pool[0].left, r.pool[0].peak);
        let mut best = 0.0f32;
        let mut t = 0.0;
        while !r.is_empty() && t < 2.0 {
            r.update(1.0 / 60.0);
            t += 1.0 / 60.0;
            if let Some(p) = r.pool.first() {
                best = best.max(p.alpha);
                if (t - 0.4 * life).abs() < 0.01 {
                    assert!(p.alpha > 0.95 * peak, "{} at its peak time", p.alpha);
                }
            }
        }
        // (a frame at the peak goes on into the fade, as in the original, so what is seen is a little under it)
        assert!((best - peak).abs() < 0.07 * peak && (t - life).abs() < 0.03, "{best} {peak} {t} {life}");
    }

    #[test]
    fn the_picture_is_laid_with_its_top_the_way_the_ripple_points_and_as_wide_as_twice_its_radius() {
        let mut r = Ripples::default();
        r.make(&walker(0.5, 2.5), State::Moving, cells);
        let ripple = &r.pool[0];
        let mut out = vec![ModelVertex { pos: [0.0; 3], normal: [0.0; 3], uv: [0.0; 2], color: [0; 4] }; 4 * POOL * CELLS];
        r.quads(1, Vec3::ZERO, &mut out);
        // Heading north (-z): a corner north of the centre has the top of the picture (v < 0.5), one to the east is to the right (u > 0.5).
        let c = ripple.centre;
        for v in out.iter().take(4) {
            let (dx, dz) = (v.pos[0] - c.x, v.pos[2] - c.z);
            assert_eq!(v.uv[1] < 0.5, dz < 0.0);
            assert_eq!(v.uv[0] > 0.5, dx > 0.0);
            assert!((v.uv[0] - 0.5 - dx / (2.0 * ripple.radius)).abs() < 1e-4);
        }
        assert_eq!(out[0].color[3], 0, "not yet bright");
        assert!(out[4 * CELLS].pos == [0.0; 3], "the rest is empty");
    }
}
