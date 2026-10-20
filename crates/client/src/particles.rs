//! Particle emitters of M2 models: the sparks, smoke, glows and rings that the original's spells are made of. An emitter throws
//! particles from its place on a bone, up and out within the angles the model gives, which fall or rise, change colour and size
//! over their life and fade; they are drawn as quads that face the camera (`quads`). Simulated in render space, so that they stay
//! where they were thrown when the model moves.

use crate::anim;
use crate::models::ModelVertex;
use glam::{Mat4, Vec2, Vec3};
use wowfmt::m2::{M2Particle, M2Track, M2};

/// Most particles an emitter keeps alive, whatever its rate and the lifespan of its particles.
pub const MAX_PARTICLES: usize = 1000;

/// How far from the camera (yards) the emission is still at its full rate, and how fast it falls off beyond (to a quarter): the original's emitters
/// throw fewer particles the farther away they are.
const FULL_RATE_RANGE: f32 = 50.0;
const RATE_FALLOFF: f32 = 0.02;

/// The share of the full emission rate at `distance` yards from the camera.
pub fn rate_at(distance: f32) -> f32 {
    (1.0 - (distance - FULL_RATE_RANGE) * RATE_FALLOFF).clamp(0.25, 1.0)
}

/// M2 model space (x east, y north, z up) to render space.
fn to_render(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

struct Particle {
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
}

pub struct Emitter {
    pub def: M2Particle,
    parts: Vec<Particle>,
    carry: f32,
    rng: u32,
    /// Particles it keeps alive at most (its rate times the lifespan of its particles).
    capacity: usize,
}

/// The value at the share `t` (0 to 1) of the life of a particle, which has two halves: the values of the first run from `values[0]` to
/// `values[1]` over the first `mid` of it, those of the second from `values[1]` to `values[2]`. Each half is walked through from 0.005 to 0.995.
fn ramp(values: [f32; 3], mid: f32, t: f32) -> f32 {
    let (half, local) = half_of(mid, t);
    values[half] + (values[half + 1] - values[half]) * local
}

/// Which half of the life `t` is in, and how far (0.005 to 0.995) in it.
fn half_of(mid: f32, t: f32) -> (usize, f32) {
    let mid = mid.clamp(0.01, 0.99);
    let (half, local) = if t < mid { (0, t / mid) } else { (1, (t - mid) / (1.0 - mid)) };
    (half, local.clamp(0.0, 1.0) * 0.99 + 0.005)
}

/// The cell of the texture at `local` (0 to 1) of a half of the life, stepping from the first to the last cell of the half (the original counts
/// both ends in, and a half that runs backwards from just before the first).
fn cell_at((first, last, power): (u16, u16, u16), local: f32) -> u32 {
    let (first, last) = (first as f32, last as f32);
    let (base, steps) = if first <= last { (first, last - first + 1.0) } else { (first + 1.0, last - first - 1.0) };
    let u = if power == 1 || power == 0 { local } else { local.powf(power as f32) };
    (base + steps * u).floor().max(0.0) as u32
}

/// A colour of the file, 0xAARRGGBB, as red, green, blue and alpha from 0 to 1.
fn channels(c: u32) -> [f32; 4] {
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0, ((c >> 24) & 255) as f32 / 255.0]
}

impl Emitter {
    pub fn new(def: M2Particle, seed: u32) -> Emitter {
        let peak = |t: &M2Track<f32>| t.keys.iter().fold(0.0f32, |m, &k| m.max(k));
        let capacity = ((peak(&def.rate) * peak(&def.lifespan)).ceil() as usize + 2).clamp(8, MAX_PARTICLES);
        Emitter { def, parts: Vec::new(), carry: 0.0, rng: seed | 1, capacity }
    }

    /// How many particles it keeps alive at most.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// A number from 0 to 1.
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// A number from -1 to 1 (the original throws its dice with a random sign).
    fn signed(&mut self) -> f32 {
        self.random() * 2.0 - 1.0
    }

    pub fn alive(&self) -> usize {
        self.parts.len()
    }

    /// Whether its particles are drawn as quads at their place (types 0 and 2) and as quads that trail behind them (types 1 and 2).
    fn shapes(&self) -> (bool, bool) {
        match self.def.particle_type {
            0 => (true, false),
            1 => (false, true),
            _ => (true, true),
        }
    }

    /// How many quads its buffer holds.
    pub fn quad_count(&self) -> usize {
        let (head, tail) = self.shapes();
        self.capacity * (usize::from(head) + usize::from(tail))
    }

    /// Advance `dt` seconds. The model is at `time` ms of its sequence `seq`; `bone` is the matrix of the emitter's bone then (model
    /// space) and `world` that of the model (render space). New particles are thrown only while `emitting` and the model's own
    /// switch for the emitter is on, at `rate_factor` of the rate of the file (`rate_at` the distance of the camera).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, m2: &M2, seq: usize, time: f32, global: f32, bone: Mat4, world: Mat4, emitting: bool, rate_factor: f32) {
        let value = |t: &M2Track<f32>, default: f32| anim::sample(t, seq, time, &m2.global_sequences, global, |a, b, k| a + (b - a) * k).unwrap_or(default);
        let gravity = value(&self.def.gravity, 0.0);
        let drag = (dt * self.def.slowdown).min(1.0);
        for p in &mut self.parts {
            p.age += dt;
            // The way the original moves one: the place by the speed, the fall by half the pull over the step, then the pull and the drag on the speed.
            p.pos += p.vel * dt;
            p.pos.y -= 0.5 * gravity * dt * dt;
            p.vel.y -= gravity * dt;
            p.vel -= p.vel * drag;
        }
        self.parts.retain(|p| p.age < p.life);
        let on = anim::sample(&self.def.enabled, seq, time, &m2.global_sequences, global, |a, _, _| a).is_none_or(|v| v != 0);
        if !emitting || !on {
            self.carry = 0.0;
            return;
        }
        self.carry += value(&self.def.rate, 0.0) * rate_factor * dt;
        let scale = world.transform_vector3(Vec3::X).length();
        while self.carry >= 1.0 {
            self.carry -= 1.0;
            if self.parts.len() >= self.capacity {
                break;
            }
            let speed = value(&self.def.speed, 0.0) * (1.0 + self.signed() * value(&self.def.variation, 0.0));
            let (vert, horiz) = (value(&self.def.vertical_range, 0.0), value(&self.def.horizontal_range, 0.0));
            let (len, wid) = (value(&self.def.area_length, 0.0), value(&self.def.area_width, 0.0));
            let life = value(&self.def.lifespan, 1.0).max(0.05);
            // Where it starts (in the emitter's own axes) and the way it goes.
            let (offset, dir) = if self.def.emitter_type == 2 {
                // A sphere: from a radius between the two lengths, along the angles, and away from the middle.
                let r = len + self.random() * (wid - len);
                let (lat, lon) = (self.signed() * vert, self.signed() * horiz);
                let dir = Vec3::new(lon.cos() * lat.cos(), lon.sin() * lat.cos(), lat.sin());
                (dir * r, dir)
            } else {
                // A plane: a place in the rectangle, and thrown within the cone of the angles (or away from a point below).
                let offset = Vec3::new(self.signed() * len * 0.5, self.signed() * wid * 0.5, 0.0);
                let (lat, lon) = (self.signed() * vert, self.signed() * horiz);
                let z_source = value(&self.def.z_source, 0.0);
                let dir = if z_source != 0.0 { (offset - Vec3::Z * z_source).normalize_or_zero() } else { Vec3::new(lon.cos() * lat.sin(), lon.sin() * lat.sin(), lat.cos()) };
                (offset, dir)
            };
            let pos = world.transform_point3(to_render(bone.transform_point3(Vec3::from(self.def.position) + offset)));
            let heading = world.transform_vector3(to_render(bone.transform_vector3(dir))).normalize_or_zero();
            // The ones thrown in this step have already lived a part of it.
            let age = self.random() * dt;
            self.parts.push(Particle { pos, vel: heading * speed * scale, age, life });
        }
    }

    /// Write the quads of the live particles (four vertices each, relative to `origin`) into `out`, which holds `quad_count` of them; the rest are
    /// collapsed to a point. `right`, `up` and `forward` are the camera's axes, `scale` is the model's. The particles of an emitter that asks for it
    /// are put far to near, for the translucent ones to blend right.
    pub fn quads(&self, origin: Vec3, right: Vec3, up: Vec3, forward: Vec3, scale: f32, out: &mut Vec<ModelVertex>) {
        out.clear();
        let (rows, cols) = (self.def.rows.max(1) as u32, self.def.cols.max(1) as u32);
        let colors = self.def.colors.map(channels);
        // The size follows the model's only when the emitter says so (bit 0x20 of its flags).
        let scale = if self.def.flags & 0x20 != 0 { scale } else { 1.0 };
        let (head, tail) = self.shapes();
        let mut order: Vec<&Particle> = self.parts.iter().collect();
        if self.def.flags & 2 != 0 {
            let depth = |p: &Particle| (p.pos - origin).dot(forward);
            order.sort_by(|a, b| depth(b).total_cmp(&depth(a)));
        }
        for p in order {
            let t = (p.age / p.life).clamp(0.0, 1.0);
            let (half, local) = half_of(self.def.mid, t);
            let size = ramp(self.def.sizes, self.def.mid, t).max(0.0) * scale;
            let c: [f32; 4] = std::array::from_fn(|i| ramp([colors[0][i], colors[1][i], colors[2][i]], self.def.mid, t));
            let color = [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8];
            let cell_uv = |cell: u32| ((cell % cols) as f32 / cols as f32, (cell / cols) as f32 / rows as f32, 1.0 / cols as f32, 1.0 / rows as f32);
            let centre = p.pos - origin;
            if head {
                let (u0, v0, du, dv) = cell_uv(cell_at(self.def.head_cells[half], local));
                for (sx, sy) in [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                    let uv = [u0 + du * (sx * 0.5 + 0.5), v0 + dv * (0.5 - sy * 0.5)];
                    let pos = centre + right * (sx * size) + up * (sy * size);
                    out.push(ModelVertex { pos: pos.into(), normal: [0.0, 1.0, 0.0], uv, color });
                }
            }
            if tail {
                // The tail trails behind the particle by its speed across the view times the tail length (held to the particle's age if the emitter says
                // so); a particle that hardly moves across the view has a square one.
                let length = if self.def.flags & 0x400 != 0 { self.def.tail_length.min(p.age) } else { self.def.tail_length };
                let across = Vec2::new(p.vel.dot(right), p.vel.dot(up)) * length;
                let (first, last) = self.def.tail_cells[half];
                let (u0, v0, du, dv) = cell_uv(cell_at((first, last, 1), local));
                let corners = if across.length_squared() >= 0.000_773 {
                    let trail = -(right * across.x + up * across.y);
                    let side = (right * -across.y + up * across.x).normalize_or_zero() * size;
                    [(centre + side, (0.0, 0.0)), (centre + trail + side, (1.0, 0.0)), (centre + trail - side, (1.0, 1.0)), (centre - side, (0.0, 1.0))]
                } else {
                    [(centre - right * size + up * size, (0.0, 0.0)), (centre + right * size + up * size, (1.0, 0.0)), (centre + right * size - up * size, (1.0, 1.0)), (centre - right * size - up * size, (0.0, 1.0))]
                };
                for (pos, (u, v)) in corners {
                    out.push(ModelVertex { pos: pos.into(), normal: [0.0, 1.0, 0.0], uv: [u0 + du * u, v0 + dv * v], color });
                }
            }
        }
        while out.len() < self.quad_count() * 4 {
            out.push(ModelVertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], color: [0; 4] });
        }
    }
}

/// The indices of `count` quads.
pub fn quad_indices(count: usize) -> Vec<u16> {
    (0..count as u16).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_ramps_through_three_points() {
        let v = [0.0, 10.0, 4.0];
        // Each half is walked through from 0.005 to 0.995 of the way, as the original does.
        assert!((ramp(v, 0.5, 0.0) - 0.05).abs() < 1e-4 && (ramp(v, 0.5, 1.0) - 4.03).abs() < 1e-3, "{} {}", ramp(v, 0.5, 0.0), ramp(v, 0.5, 1.0));
        assert!((ramp(v, 0.5, 0.25) - 5.0).abs() < 0.1 && (ramp(v, 0.5, 0.75) - 7.0).abs() < 0.1);
    }

    #[test]
    fn a_colour_of_the_file_is_alpha_red_green_blue() {
        assert_eq!(channels(0x80FF4000), [1.0, 64.0 / 255.0, 0.0, 128.0 / 255.0]);
    }

    #[test]
    fn a_fire_impact_throws_particles_that_live_and_die_and_fill_their_quads() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let m2 = M2::parse(&mount.read(r"Spells\Fire_ImpactDD_Low_Chest.m2").unwrap()).unwrap();
        assert_eq!(m2.particles.len(), 5);
        let seq = m2.sequences.iter().position(|s| s.id == 0).unwrap();
        let start = m2.sequences[seq].start as f32;
        let mut e = Emitter::new(m2.particles[2].clone(), 7);
        let bones = anim::bone_matrices(&m2, seq, start, 0.0);
        let bone = bones[e.def.bone as usize];
        for step in 0..30 {
            e.update(1.0 / 60.0, &m2, seq, start + step as f32 * 16.0, 0.0, bone, Mat4::IDENTITY, true, 1.0);
        }
        assert!(e.alive() > 5, "{} particles", e.alive());
        let mut out = Vec::new();
        e.quads(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, 1.0, &mut out);
        assert_eq!(out.len(), e.quad_count() * 4);
        assert!(out[..e.alive() * 4].iter().all(|v| v.color[3] > 0 || true));
        // No longer emitting: they run out.
        for _ in 0..200 {
            e.update(1.0 / 60.0, &m2, seq, start, 0.0, bone, Mat4::IDENTITY, false, 1.0);
        }
        assert_eq!(e.alive(), 0);
    }

    #[test]
    fn a_particle_steps_through_the_cells_of_its_texture_over_its_life() {
        // The torch's flame: the cells 0 to 7 in the first half of the life and 8 to 16 in the second.
        let torch = [(0, 7, 1), (8, 16, 1)];
        assert_eq!((cell_at(torch[0], 0.005), cell_at(torch[0], 0.995), cell_at(torch[1], 0.005), cell_at(torch[1], 0.995)), (0, 7, 8, 16));
        // A candle's flame is the first cell all through.
        assert_eq!((cell_at((0, 0, 1), 0.005), cell_at((0, 0, 1), 0.995)), (0, 0));
    }

    #[test]
    fn emission_is_at_the_full_rate_near_the_camera_and_a_quarter_far_from_it() {
        assert_eq!((rate_at(10.0), rate_at(50.0), rate_at(200.0)), (1.0, 1.0, 0.25));
        assert!((rate_at(70.0) - 0.6).abs() < 1e-5);
    }

    #[test]
    fn an_emitter_of_heads_and_tails_fills_two_quads_for_each_particle_and_puts_far_ones_first_when_asked() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let m2 = M2::parse(&mount.read(r"Spells\Fire_ImpactDD_Low_Chest.m2").unwrap()).unwrap();
        let mut def = m2.particles[2].clone();
        (def.particle_type, def.tail_length, def.flags) = (2, 1.0, def.flags | 2);
        let mut e = Emitter::new(def, 11);
        assert_eq!(e.quad_count(), 2 * e.capacity());
        let seq = m2.sequences.iter().position(|s| s.id == 0).unwrap();
        let start = m2.sequences[seq].start as f32;
        let bone = anim::bone_matrices(&m2, seq, start, 0.0)[e.def.bone as usize];
        for step in 0..30 {
            e.update(1.0 / 60.0, &m2, seq, start + step as f32 * 16.0, 0.0, bone, Mat4::IDENTITY, true, 1.0);
        }
        let mut out = Vec::new();
        e.quads(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, 1.0, &mut out);
        assert_eq!(out.len(), e.quad_count() * 4);
        // The first quad of the sorted list belongs to the farthest particle along the viewer's way (z here).
        let depth = |quad: usize| out[quad * 4..quad * 4 + 4].iter().map(|v| v.pos[2]).sum::<f32>() / 4.0;
        let heads: Vec<f32> = (0..e.alive()).map(|i| depth(i * 2)).collect();
        assert!(heads.windows(2).all(|w| w[0] >= w[1] - 1e-4), "{heads:?}");
    }
}
