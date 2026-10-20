//! Effects drawn in the world: the original's own models of `Spells\` (the golden rings that rise round a character who gains a
//! level, the glows, missiles and bursts of the spells), each a mesh with a bone animation and, mostly, particle emitters
//! (`particles.rs`). An effect is placed with a matrix in render space that the game moves as it likes (a missile flies, a glow
//! follows a hand), and plays until its animation is over and its particles have died, or, if it loops, until it is stopped.

use crate::anim;
use crate::camera::Camera;
use crate::models::{BatchCpu, ModelCache, ModelCpu, ModelVertex, RawTexture};
use crate::particles::{quad_indices, rate_at, Emitter};
use crate::renderer::{Gpu, Renderer};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::collections::HashMap;
use std::sync::Arc;
use wowfmt::m2::{Blend, M2};

pub const LEVEL_UP: &str = r"Spells\LevelUp\LevelUp.m2";
/// Fraction of the animation over which a mesh fades in and out. The models animate their own opacity in tracks the game does not
/// read, so the whole mesh fades instead.
/// Yards within which an effect is always worked out, whichever way the viewer looks.
const SEEN_ANYWAY: f32 = 10.0;
const FADE_IN: f32 = 0.12;
const FADE_OUT: f32 = 0.4;

/// How bright a mesh is at `t` (0 to 1) through its animation.
fn envelope(t: f32) -> f32 {
    (t / FADE_IN).min((1.0 - t) / FADE_OUT).clamp(0.0, 1.0)
}

/// A model of effects, loaded once.
struct Model {
    m2: M2,
    /// The mesh, if the model has any (many are particles alone).
    template: Option<Arc<ModelCpu>>,
    seq: usize,
    start: f32,
    length: f32,
    /// The texture of each particle emitter.
    textures: Vec<Option<(String, Arc<RawTexture>)>>,
    /// Some of its bones are billboards, which face the viewer.
    billboards: bool,
}

struct Rig {
    emitter: Emitter,
    entity: usize,
    vertices: Vec<ModelVertex>,
}

/// An effect that is playing, or waiting to be played again.
struct Instance {
    model: usize,
    mesh: Option<usize>,
    rigs: Vec<Rig>,
    /// Milliseconds since it began.
    age: f32,
    looping: bool,
    emitting: bool,
    active: bool,
    /// Not shown and not emitting for now (what carries it is put away).
    hidden: bool,
    /// Drawn lit by the world's light (a placed model) instead of shining at the brightness of the moment (an effect), or by the light of its building.
    lit: bool,
    light: Option<crate::models::InteriorLight>,
    world: Mat4,
    vertices: Vec<ModelVertex>,
}

pub struct Effects {
    /// The share of the particles of the file that is made (the spell detail of the video options).
    pub detail: f32,
    /// Where the models come from (`None` for effects that are given a mount at each spawn).
    mount: Option<Arc<Mount>>,
    cache: ModelCache,
    models: Vec<Model>,
    by_path: HashMap<String, usize>,
    instances: Vec<Instance>,
    seed: u32,
}

impl Effects {
    pub fn new(mount: Arc<Mount>) -> Effects {
        Effects { mount: Some(mount), ..Effects::unmounted() }
    }

    /// Effects that read their models from the mount given to `spawn_in`.
    pub fn unmounted() -> Effects {
        Effects { mount: None, cache: ModelCache::default(), models: Vec::new(), by_path: HashMap::new(), instances: Vec::new(), seed: 0x5EED_0001, detail: 1.0 }
    }

    fn model(&mut self, mount: &Mount, path: &str, skin: Option<&str>, mesh: bool) -> Option<usize> {
        let key = format!("{}|{}", path.to_ascii_lowercase(), skin.unwrap_or("").to_ascii_lowercase());
        if let Some(&i) = self.by_path.get(&key) {
            return Some(i);
        }
        let result = (|| -> anyhow::Result<Model> {
            let m2 = M2::parse(&mount.read(path)?)?;
            let template = if !mesh || m2.vertices.is_empty() { None } else { Some(match skin {
                Some(skin) => self.cache.item(mount, path, skin)?,
                None => self.cache.get(mount, path).ok_or_else(|| anyhow::anyhow!("no model"))?,
            })
            };
            let seq = m2.sequences.iter().position(|s| s.id == 0).or(if m2.sequences.is_empty() { None } else { Some(0) });
            let (start, length) = seq.map_or((0.0, 1000.0), |s| (m2.sequences[s].start as f32, ((m2.sequences[s].end - m2.sequences[s].start) as f32).max(1.0)));
            let textures = m2.particles.iter().map(|p| m2.textures.get(p.texture as usize).and_then(|t| self.cache.texture(mount, &t.name))).collect();
            let billboards = anim::has_billboards(&m2);
            Ok(Model { m2, template, seq: seq.unwrap_or(0), start, length, textures, billboards })
        })();
        match result {
            Ok(model) => {
                self.models.push(model);
                self.by_path.insert(key, self.models.len() - 1);
                Some(self.models.len() - 1)
            }
            Err(e) => {
                log::warn!("no effect {path}: {e:#}");
                None
            }
        }
    }

    /// Start the effect `path` standing at `world` (render space). It plays once and ends, or, if `looping`, until `stop`.
    pub fn spawn(&mut self, renderer: &mut Renderer, gpu: &Gpu, path: &str, looping: bool, world: Mat4) -> Option<usize> {
        let mount = self.mount.clone()?;
        self.spawn_in(&mount, renderer, gpu, path, None, looping, true, world)
    }

    /// Like `spawn`, for a model of the items with the texture `skin` painted on it (an arrow in flight).
    pub fn spawn_skinned(&mut self, renderer: &mut Renderer, gpu: &Gpu, path: &str, skin: &str, looping: bool, world: Mat4) -> Option<usize> {
        let mount = self.mount.clone()?;
        self.spawn_in(&mount, renderer, gpu, path, Some(skin), looping, true, world)
    }

    /// A model of the world that lives (flames and glows that turn to the viewer, a flickering lamp) and its particles, shown lit like the rest of the
    /// world, in the place of the still one: it plays its stand animation until `stop`.
    pub fn spawn_placed(&mut self, renderer: &mut Renderer, gpu: &Gpu, path: &str, light: Option<crate::models::InteriorLight>, world: Mat4) -> Option<usize> {
        let mount = self.mount.clone()?;
        let id = self.spawn_in(&mount, renderer, gpu, path, None, true, true, world)?;
        self.instances[id].lit = true;
        self.instances[id].light = light;
        Some(id)
    }

    /// The particle emitters of the model `path` (a placed torch or fountain), burning until `stop`: the mesh, if it has one, is drawn by whoever placed it.
    pub fn spawn_emitters(&mut self, renderer: &mut Renderer, gpu: &Gpu, path: &str, world: Mat4) -> Option<usize> {
        let mount = self.mount.clone()?;
        self.spawn_in(&mount, renderer, gpu, path, None, true, false, world)
    }

    /// Like `spawn`, for a model of `mount`; without `mesh` only the particles of the model are shown (the emitters of a weapon).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_in(&mut self, mount: &Mount, renderer: &mut Renderer, gpu: &Gpu, path: &str, skin: Option<&str>, looping: bool, mesh: bool, world: Mat4) -> Option<usize> {
        let model = self.model(mount, path, skin, mesh)?;
        let id = match self.instances.iter().position(|i| !i.active && i.model == model) {
            Some(id) => id,
            None => {
                let m = &self.models[model];
                let mesh = m.template.as_ref().map(|t| renderer.add_entity(gpu, t));
                let mut rigs = Vec::new();
                for (k, def) in m.m2.particles.iter().enumerate() {
                    // The original's modes for a particle: the file's 1, 2, 4, 5 and 6 are its alpha key, alpha, additive, modulate and twice modulate;
                    // 0 and 3 are drawn without blending there (here a cut-out and an additive one: nothing of the game uses them).
                    let blend = match def.blend {
                        0 | 1 => Blend::AlphaKey,
                        3 | 4 => Blend::Additive,
                        5 => Blend::Modulate,
                        6 => Blend::Modulate2x,
                        _ => Blend::Alpha,
                    };
                    let key = format!("particles/{path}/{}/{k}", self.instances.len());
                    let emitter = Emitter::new(def.clone(), 0);
                    let capacity = emitter.quad_count();
                    let cpu = ModelCpu {
                        key,
                        vertices: vec![ModelVertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], uv: [0.0; 2], color: [0; 4] }; capacity * 4],
                        indices: quad_indices(capacity),
                        batches: vec![BatchCpu { index_start: 0, index_count: (capacity * 6) as u32, texture: m.textures[k].clone(), wrap: false, blend, unlit: true, refs: Default::default(), particle: true }],
                        col_vertices: Vec::new(),
                        col_indices: Vec::new(),
                        tracks: None,
                    };
                    let entity = renderer.add_entity(gpu, &cpu);
                    renderer.set_entity(entity, Vec3::ZERO, Mat4::IDENTITY, false);
                    self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    rigs.push(Rig { emitter: Emitter::new(def.clone(), self.seed), entity, vertices: Vec::new() });
                }
                if let Some(mesh) = mesh {
                    renderer.set_entity(mesh, Vec3::ZERO, Mat4::IDENTITY, false);
                }
                self.instances.push(Instance { model, mesh, rigs, age: 0.0, looping, emitting: true, active: false, hidden: false, lit: false, light: None, world, vertices: Vec::new() });
                self.instances.len() - 1
            }
        };
        let inst = &mut self.instances[id];
        (inst.age, inst.looping, inst.emitting, inst.active, inst.hidden, inst.world) = (0.0, looping, true, true, false, world);
        (inst.lit, inst.light) = (false, None);
        for rig in &mut inst.rigs {
            rig.emitter = Emitter::new(rig.emitter.def.clone(), self.seed.wrapping_add(id as u32 * 977 + 13));
        }
        Some(id)
    }

    pub fn set_world(&mut self, id: usize, world: Mat4) {
        if let Some(i) = self.instances.get_mut(id) {
            i.world = world;
        }
    }

    /// Hide a looping effect and stop its particles, or show it again.
    pub fn set_hidden(&mut self, id: usize, hidden: bool) {
        if let Some(i) = self.instances.get_mut(id) {
            i.hidden = hidden;
        }
    }

    /// Every entity of every effect.
    pub fn entities(&self) -> Vec<usize> {
        self.instances.iter().flat_map(|i| i.mesh.into_iter().chain(i.rigs.iter().map(|r| r.entity))).collect()
    }

    /// Stop a looping effect: its particles are not renewed and it ends when the last of them is gone.
    pub fn stop(&mut self, id: usize) {
        if let Some(i) = self.instances.get_mut(id) {
            (i.looping, i.emitting, i.hidden) = (false, false, false);
            i.age = i.age.max(self.models[i.model].length);
        }
    }

    /// How many particles the effect has alive.
    pub fn particles(&self, id: usize) -> usize {
        self.instances.get(id).map_or(0, |i| i.rigs.iter().map(|r| r.emitter.alive()).sum())
    }

    pub fn active(&self, id: usize) -> bool {
        self.instances.get(id).is_some_and(|i| i.active)
    }

    /// Advance everything `dt` seconds, drawn to face `camera`.
    pub fn update(&mut self, dt: f32, camera: &Camera, renderer: &mut Renderer, gpu: &Gpu) {
        self.update_facing(dt, camera.forward(), camera.pos, renderer, gpu);
    }

    /// Like `update`, with the particles drawn to face `forward` (the way the viewer looks) and thrown at the rate that fits the distance of `viewer`.
    pub fn update_facing(&mut self, dt: f32, forward: Vec3, viewer: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward);
        for inst in &mut self.instances {
            if !inst.active {
                continue;
            }
            let m = &self.models[inst.model];
            inst.age += dt * 1000.0;
            // A fire that burns out of the viewer's sight (behind him, or off to a side) is not worked out and not drawn: it only grows older.
            let seen_at = inst.world.w_axis.truncate();
            let (to, near) = (seen_at - viewer, seen_at.distance(viewer) < SEEN_ANYWAY);
            if inst.looping && !near && to.dot(forward) < 0.25 * to.length() {
                for entity in inst.mesh.into_iter().chain(inst.rigs.iter().map(|r| r.entity)) {
                    renderer.set_entity(entity, seen_at, Mat4::IDENTITY, false);
                }
                continue;
            }
            let playing = !inst.hidden && (inst.looping || inst.age < m.length);
            let time = if inst.looping { inst.age % m.length } else { inst.age.min(m.length) };
            let bones = if m.billboards {
                // The viewer in the model's own axes: render space, undone by the placement of the model, then to the model's z up.
                let there = inst.world.inverse().transform_point3(viewer);
                anim::bone_matrices_facing(&m.m2, m.seq, m.start + time, inst.age, Vec3::new(there.x, -there.z, there.y))
            } else {
                anim::bone_matrices(&m.m2, m.seq, m.start + time, inst.age)
            };
            let origin = inst.world.w_axis.truncate();
            let scale = inst.world.transform_vector3(Vec3::X).length();
            let orient = Mat4::from_cols(inst.world.x_axis, inst.world.y_axis, inst.world.z_axis, glam::Vec4::W);
            if let Some(mesh) = inst.mesh {
                if playing {
                    anim::skin(&m.m2, &bones, &m.template.as_ref().expect("a mesh has its template").vertices, &mut inst.vertices);
                    // Vertex colour above half alpha replaces the lighting: grey of the brightness of the moment.
                    // The model's own tracks fade it in and out; one without them is faded as a whole.
                    let tracks = m.template.as_ref().and_then(|t| t.tracks.as_ref());
                    let k = (if inst.looping || tracks.is_some_and(|t| t.animated) { 1.0 } else { envelope(inst.age / m.length) } * 255.0) as u8;
                    let color = if inst.lit { [255, 255, 255, 0] } else { [k, k, k, 255] };
                    inst.vertices.iter_mut().for_each(|v| v.color = color);
                    renderer.update_entity_vertices(gpu, mesh, &inst.vertices);
                    if let (Some(tracks), Some(template)) = (tracks.filter(|t| t.animated), &m.template) {
                        let looks: Vec<_> = template.batches.iter().map(|b| tracks.look(b.refs, inst.age)).collect();
                        renderer.set_entity_looks(gpu, mesh, &looks);
                    }
                }
                renderer.set_entity(mesh, origin, orient, playing);
                renderer.set_entity_light(mesh, inst.light);
            }
            let mut alive = 0;
            for rig in &mut inst.rigs {
                let bone = bones.get(rig.emitter.def.bone as usize).copied().unwrap_or(Mat4::IDENTITY);
                rig.emitter.update(dt, &m.m2, m.seq, m.start + time, inst.age, bone, inst.world, inst.emitting && playing, rate_at(origin.distance(viewer)) * self.detail);
                rig.emitter.quads(origin, right, up, forward, scale, &mut rig.vertices);
                renderer.update_entity_vertices(gpu, rig.entity, &rig.vertices);
                renderer.set_entity(rig.entity, origin, Mat4::IDENTITY, rig.emitter.alive() > 0);
                alive += rig.emitter.alive();
            }
            if !playing && alive == 0 && !inst.hidden {
                inst.active = false;
                if let Some(mesh) = inst.mesh {
                    renderer.set_entity(mesh, origin, orient, false);
                }
                for rig in &inst.rigs {
                    renderer.set_entity(rig.entity, origin, Mat4::IDENTITY, false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mesh_fades_in_and_out_and_shines_between() {
        assert_eq!(envelope(0.0), 0.0);
        assert_eq!(envelope(1.0), 0.0);
        assert!(envelope(FADE_IN) >= 0.99 && envelope(0.5) == 1.0);
        assert!(envelope(0.06) > 0.4 && envelope(0.06) < 0.6);
    }
}
