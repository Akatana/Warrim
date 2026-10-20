//! The pictures of the editor's model gallery: every doodad and building of the game, seen from the front and a little above. A model is
//! loaded on a thread of its own when the gallery first shows it, and drawn once into one of a few small offscreen pictures; a picture
//! is taken back from the models scrolled out of sight when another one needs it.

use crate::camera::Camera;
use crate::character::Appearance;
use crate::models::{ModelCache, ModelCpu, WmoCpu};
use crate::propedit::Models;
use crate::renderer::{Gpu, Renderer};
use crate::world::World;
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// The pictures' size in pixels, how many there are, how many models may be loading at once and how many are drawn in a frame.
const SIZE: (u32, u32) = (128, 128);
const SLOTS: usize = 64;
const IN_FLIGHT: usize = 4;
const PER_FRAME: usize = 2;

/// What the gallery has to show of a model now.
pub enum Pic {
    /// The interface texture of its picture.
    Ready(usize),
    Loading,
    /// It has nothing to see (effects only, or it cannot be read).
    Empty,
}

/// What a picture is made of when it is not a model file of the archives: a creature, seen standing.
#[derive(Clone)]
pub enum Subject {
    Beast { model: String, skins: Vec<(u32, String)> },
    Person { outfit: Vec<usize>, look: Appearance },
}

enum Loaded {
    Doodad(Option<Arc<ModelCpu>>),
    Building(Arc<WmoCpu>),
}

enum Thumb {
    Slot(usize),
    Empty,
}

struct Slot {
    target: usize,
    texture: usize,
    owner: Option<String>,
    used: u64,
}

pub struct Gallery {
    requests: Sender<String>,
    results: Receiver<(String, Loaded)>,
    slots: Vec<Slot>,
    thumbs: HashMap<String, Thumb>,
    pending: HashSet<String>,
    /// The models the window shows (asked for since `begin`).
    visible: Vec<String>,
    /// What the pictures that are made here (not by the loading thread) show, by their key.
    local: HashMap<String, Subject>,
    frame: u64,
}

impl Gallery {
    pub fn new(mount: Arc<Mount>, cache: Arc<ModelCache>) -> Gallery {
        let (requests, wanted) = channel::<String>();
        let (done, results) = channel();
        std::thread::spawn(move || {
            while let Ok(file) = wanted.recv() {
                let loaded = if file.to_ascii_lowercase().ends_with(".wmo") { Loaded::Building(cache.get_wmo(&mount, &file)) } else { Loaded::Doodad(cache.get(&mount, &file)) };
                if done.send((file, loaded)).is_err() {
                    break;
                }
            }
        });
        Gallery { requests, results, slots: Vec::new(), thumbs: HashMap::new(), pending: HashSet::new(), visible: Vec::new(), local: HashMap::new(), frame: 0 }
    }

    /// A frame of the window begins: nothing is shown yet.
    pub fn begin(&mut self) {
        self.visible.clear();
    }

    /// The picture of a model, which the window shows this frame.
    pub fn picture(&mut self, file: &str) -> Pic {
        self.visible.push(file.to_string());
        match self.thumbs.get(file) {
            Some(Thumb::Slot(s)) => {
                self.slots[*s].used = self.frame;
                Pic::Ready(self.slots[*s].texture)
            }
            Some(Thumb::Empty) => Pic::Empty,
            None => Pic::Loading,
        }
    }

    /// The picture of a creature (or whatever else is made from a `Subject`), which the window shows this frame.
    pub fn picture_subject(&mut self, key: &str, subject: &Subject) -> Pic {
        self.local.entry(key.to_string()).or_insert_with(|| subject.clone());
        self.picture(key)
    }

    /// Whether models are still being loaded for the window.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || self.visible.iter().any(|f| !self.thumbs.contains_key(f))
    }

    /// Ask for the models on show and draw those that were loaded. `base` is anywhere on a loaded tile of the world.
    pub fn update(&mut self, renderer: &mut Renderer, gpu: &Gpu, world: &World, base: Vec3, models: &mut Models) {
        self.frame += 1;
        if self.slots.is_empty() {
            self.slots = (0..SLOTS)
                .map(|_| {
                    let (target, texture) = renderer.add_offscreen(gpu, SIZE);
                    Slot { target, texture, owner: None, used: 0 }
                })
                .collect();
        }
        for file in &self.visible {
            if self.local.contains_key(file) {
                continue;
            }
            if self.pending.len() < IN_FLIGHT && !self.thumbs.contains_key(file) && !self.pending.contains(file) {
                self.pending.insert(file.clone());
                let _ = self.requests.send(file.clone());
            }
        }
        // The creatures are drawn here, a few in a frame.
        let todo: Vec<String> = self.visible.iter().filter(|k| self.local.contains_key(*k) && !self.thumbs.contains_key(*k)).take(PER_FRAME).cloned().collect();
        for key in todo {
            self.paint_creature(renderer, gpu, world, base, models, key);
        }
        for _ in 0..PER_FRAME {
            let Ok((file, loaded)) = self.results.try_recv() else { break };
            self.pending.remove(&file);
            self.paint(renderer, gpu, world, base, file, loaded);
        }
    }

    /// The slot to draw into: a free one, else the one that was shown longest ago.
    fn take_slot(&mut self) -> usize {
        let at = self.slots.iter().position(|s| s.owner.is_none()).unwrap_or_else(|| (0..self.slots.len()).min_by_key(|&i| self.slots[i].used).expect("there are slots"));
        if let Some(old) = self.slots[at].owner.take() {
            self.thumbs.remove(&old);
        }
        at
    }

    /// Draw a creature standing, seen from the front, into a picture.
    fn paint_creature(&mut self, renderer: &mut Renderer, gpu: &Gpu, world: &World, base: Vec3, models: &mut Models, key: String) {
        let Some(subject) = self.local.get(&key).cloned() else { return };
        // As a creature faces north when it is put down: the picture is taken from the north-east.
        let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let (ids, bounds, person) = match subject {
            Subject::Beast { model, skins } => match models.beast_entity(renderer, gpu, &model, &skins) {
                Ok((entity, vertices)) => {
                    let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
                    for v in &vertices {
                        for i in 0..3 {
                            (low[i], high[i]) = (low[i].min(v.pos[i]), high[i].max(v.pos[i]));
                        }
                    }
                    renderer.set_entity(entity, base, orient, true);
                    (vec![entity], (Vec3::from(low), Vec3::from(high)), None)
                }
                Err(e) => {
                    log::warn!("no picture of {model}: {e:#}");
                    self.thumbs.insert(key, Thumb::Empty);
                    return;
                }
            },
            Subject::Person { outfit, look } => match models.person(renderer, gpu, &outfit, &look, &key) {
                Ok(mut person) => {
                    let ids = person.stand_facing(0.0, base, 0.0, renderer, gpu);
                    (ids, (Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 2.0, 0.5)), Some(person))
                }
                Err(e) => {
                    log::warn!("no picture of a person: {e:#}");
                    self.thumbs.insert(key, Thumb::Empty);
                    return;
                }
            },
        };
        let slot = self.take_slot();
        renderer.render_offscreen(gpu, self.slots[slot].target, &frame(bounds.0, bounds.1, orient, base), &ids, world.tiles());
        match person {
            Some(person) => person.retire(renderer),
            None => ids.into_iter().for_each(|id| renderer.remove_entity(id)),
        }
        self.slots[slot].owner = Some(key.clone());
        self.slots[slot].used = self.frame;
        self.thumbs.insert(key, Thumb::Slot(slot));
    }

    fn paint(&mut self, renderer: &mut Renderer, gpu: &Gpu, world: &World, base: Vec3, file: String, loaded: Loaded) {
        let models: Vec<Arc<ModelCpu>> = match loaded {
            Loaded::Doodad(model) => model.into_iter().collect(),
            Loaded::Building(wmo) => wmo.groups.iter().filter_map(|g| g.model.clone()).collect(),
        };
        let models: Vec<Arc<ModelCpu>> = models.into_iter().filter(|m| !m.vertices.is_empty()).collect();
        if models.is_empty() {
            self.thumbs.insert(file, Thumb::Empty);
            return;
        }
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in models.iter().flat_map(|m| &m.vertices) {
            for i in 0..3 {
                (low[i], high[i]) = (low[i].min(v.pos[i]), high[i].max(v.pos[i]));
            }
        }
        let slot = self.take_slot();
        let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let ids: Vec<usize> = models
            .iter()
            .map(|m| {
                let id = renderer.add_entity(gpu, m);
                renderer.set_entity(id, base, orient, true);
                id
            })
            .collect();
        renderer.render_offscreen(gpu, self.slots[slot].target, &frame(Vec3::from(low), Vec3::from(high), orient, base), &ids, world.tiles());
        for id in ids {
            renderer.remove_entity(id);
        }
        self.slots[slot].owner = Some(file.clone());
        self.slots[slot].used = self.frame;
        self.thumbs.insert(file, Thumb::Slot(slot));
    }
}

/// A camera that sees all of a model (its box `low`..`high`, turned by `orient`, standing at `base`) from the front, a little turned and above.
fn frame(low: Vec3, high: Vec3, orient: Mat4, base: Vec3) -> Camera {
    const TURN: f32 = 0.6;
    const RISE: f32 = 0.35;
    let fov = 25f32.to_radians();
    let around = std::f32::consts::PI + TURN;
    let (forward, right) = (Vec3::new(around.sin(), 0.0, -around.cos()), Vec3::new(around.cos(), 0.0, around.sin()));
    let corners: Vec<Vec3> = (0..8).map(|i| orient.transform_point3(Vec3::new(if i & 1 == 0 { low.x } else { high.x }, if i & 2 == 0 { low.y } else { high.y }, if i & 4 == 0 { low.z } else { high.z }))).collect();
    let middle = corners.iter().fold(Vec3::ZERO, |m, c| m + *c / 8.0);
    let (mut half_w, mut half_h, mut half_d) = (0f32, 0f32, 0f32);
    for c in &corners {
        let c = *c - middle;
        (half_w, half_h, half_d) = (half_w.max(c.dot(right).abs()), half_h.max(c.y.abs()), half_d.max(c.dot(forward).abs()));
    }
    // The picture is square; looking down a little, the depth of the model adds to its height on the picture.
    let tan = (fov / 2.0).tan();
    let distance = ((half_h + half_d * RISE) / tan).max(half_w / tan) * 1.1 + half_d;
    let mut camera = Camera::new(base + middle - forward * distance + Vec3::Y * distance * RISE, around, -RISE.atan());
    (camera.fov_y, camera.aspect, camera.near, camera.far) = (fov, 1.0, 0.1, distance * 2.0 + half_d + 20.0);
    camera
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_sees_the_whole_box_of_a_model_however_big() {
        for size in [0.3, 2.0, 40.0] {
            let (low, high) = (Vec3::new(-size, 0.0, -size / 2.0), Vec3::new(size, size * 2.0, size / 2.0));
            let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
            let camera = frame(low, high, orient, Vec3::ZERO);
            let clip = camera.view_proj_relative();
            for i in 0..8 {
                let corner = orient.transform_point3(Vec3::new(if i & 1 == 0 { low.x } else { high.x }, if i & 2 == 0 { low.y } else { high.y }, if i & 4 == 0 { low.z } else { high.z }));
                let p = clip * (corner - camera.pos).extend(1.0);
                assert!(p.w > 0.0 && (p.x / p.w).abs() <= 1.0 && (p.y / p.w).abs() <= 1.0, "size {size}, corner {i}: {p}");
            }
        }
    }
}
