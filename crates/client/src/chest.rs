//! Chests standing in the world (`content/chests.ron`): closed, locked or not; opened by hand or by picking the lock, they
//! swing their lid up (the model's own opening animation) and hold loot to take. Whether each is open or still locked and
//! what is left in it is part of the saved game (`ChestSave`).

use crate::anim;
use crate::content::{self, ChestDef};
use crate::lockpick::Level;
use crate::items::Stack;
use crate::loot::Loot;
use crate::models::{ModelCache, ModelCpu, ModelVertex};
use crate::renderer::{Gpu, Renderer};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::sync::Arc;
use wowfmt::m2::M2;

/// Animation ids of a game object: closed (a creature's stand, or the closed pose) and opening.
const STAND: u16 = 0;
const CLOSED: u16 = 147;
const OPENING: u16 = 148;

/// How far from its place the chests' loot windows stay open and from where they can be opened, yards.
pub const OPEN_RANGE: f32 = 3.5;

struct Rig {
    m2: M2,
    template: Arc<ModelCpu>,
    closed: usize,
    opening: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Closed,
    /// Milliseconds into the opening animation.
    Opening(f32),
    Open,
}

struct Chest {
    def: &'static ChestDef,
    rig: usize,
    pos: Vec3,
    yaw: f32,
    entity: usize,
    state: State,
    /// How hard the lock is while it is still locked.
    locked: Option<Level>,
    /// Rolled when the chest is first opened, then what is left in it.
    loot: Option<Loot>,
    /// Whether it stands on the ground yet (the terrain may not be loaded when the game starts).
    placed: bool,
}

/// A chest as saved: which it is, whether it was opened or is still locked, and what is left in it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChestSave {
    id: String,
    open: bool,
    locked: bool,
    loot: Option<crate::save::LootSave>,
}

pub struct Chests {
    rigs: Vec<Rig>,
    chests: Vec<Chest>,
    vertices: Vec<ModelVertex>,
    rng: u32,
}

/// What a chest holds when it is opened: what its drop table rolls, and what is fixed (more than one stack of an item when it is more than
/// fits on one). Nothing when that is nothing.
fn contents(def: &ChestDef, rng: &mut u32, wanted: &[usize]) -> Option<Loot> {
    let mut loot = content::drop_table(&def.loot).map(|t| t.roll(rng)).unwrap_or_default();
    loot.copper += def.copper;
    for (id, count) in &def.items {
        let item = crate::items::find(id);
        let mut left = *count;
        while left > 0 {
            let n = left.min(crate::items::get(item).stack);
            loot.items.push(Stack { item, count: n, enchant: 0 });
            left -= n;
        }
    }
    loot.only_wanted(wanted);
    Some(loot).filter(|l| !l.is_empty())
}

fn sequence(m2: &M2, ids: &[u16]) -> Option<usize> {
    ids.iter().find_map(|&id| m2.sequences.iter().position(|s| s.id == id))
}

impl Chests {
    /// Take the chests out of the world, giving their entities back: the world is made again from new content.
    pub fn retire(self, renderer: &mut Renderer) {
        self.chests.iter().for_each(|c| renderer.remove_entity(c.entity));
    }

    pub fn load(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu) -> Result<Chests> {
        let mut rigs: Vec<(String, Rig)> = Vec::new();
        let mut chests = Vec::new();
        for def in &content::get().chests {
            let at = match rigs.iter().position(|(path, _)| *path == def.model) {
                Some(i) => i,
                None => {
                    let m2 = M2::parse(&mount.read(&def.model)?).with_context(|| format!("chest model {}", def.model))?;
                    let template = cache.get(mount, &def.model).with_context(|| format!("chest model {}", def.model))?;
                    let (closed, opening) = (sequence(&m2, &[STAND, CLOSED]).context("a chest model needs a closed pose")?, sequence(&m2, &[OPENING]).context("a chest model needs an opening animation")?);
                    rigs.push((def.model.clone(), Rig { m2, template, closed, opening }));
                    rigs.len() - 1
                }
            };
            let entity = renderer.add_entity(gpu, &rigs[at].1.template);
            let pos = crate::coords::wow_to_render([def.at.0, def.at.1, 0.0]);
            chests.push(Chest { def, rig: at, pos, yaw: def.facing.to_radians(), entity, state: State::Closed, locked: def.lock, loot: None, placed: false });
        }
        Ok(Chests { rigs: rigs.into_iter().map(|(_, r)| r).collect(), chests, vertices: Vec::new(), rng: 0xC0FF_EE11 })
    }

    /// Place the chests on the ground once it is there, and play the opening of those being opened. `dt` in seconds.
    pub fn update(&mut self, dt: f32, ground: impl Fn(f32, f32) -> Option<f32>, renderer: &mut Renderer, gpu: &Gpu) {
        for c in &mut self.chests {
            let rig = &self.rigs[c.rig];
            let pose = |at: usize, time: f32, vertices: &mut Vec<ModelVertex>| {
                let bones = anim::bone_matrices(&rig.m2, at, time, time);
                anim::skin(&rig.m2, &bones, &rig.template.vertices, vertices);
            };
            let mut changed = false;
            if !c.placed {
                let Some(y) = ground(c.pos.x, c.pos.z) else { continue };
                c.pos.y = y;
                c.placed = true;
                changed = true;
            }
            if let State::Opening(age) = &mut c.state {
                *age += dt * 1000.0;
                changed = true;
            }
            if changed {
                let seq = match c.state {
                    State::Closed => rig.closed,
                    _ => rig.opening,
                };
                let s = &rig.m2.sequences[seq];
                let at = match c.state {
                    State::Closed => 0.0,
                    State::Opening(age) => age.min((s.end - s.start) as f32),
                    State::Open => (s.end - s.start) as f32,
                };
                if matches!(c.state, State::Opening(age) if age >= (s.end - s.start) as f32) {
                    c.state = State::Open;
                }
                pose(seq, s.start as f32 + at, &mut self.vertices);
                renderer.update_entity_vertices(gpu, c.entity, &self.vertices);
                renderer.set_entity(c.entity, c.pos, Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.yaw), true);
            }
        }
    }

    /// The nearest chest within `range` yards of `from` (feet, render space) that has something to give: closed, or open with loot.
    /// Only chests `seen` accepts (given where it stands) count.
    pub fn near(&self, from: Vec3, range: f32, seen: impl Fn(Vec3) -> bool) -> Option<usize> {
        let d = |c: &Chest| Vec3::new(c.pos.x - from.x, 0.0, c.pos.z - from.z).length();
        (0..self.chests.len())
            .filter(|&i| d(&self.chests[i]) <= range && (self.chests[i].state == State::Closed || self.chests[i].loot.is_some()) && seen(self.chests[i].pos))
            .min_by(|&a, &b| d(&self.chests[a]).total_cmp(&d(&self.chests[b])))
    }

    /// The chest under the pointer at `pos` (pixels) on a `size` screen seen through `camera`, with the pointer that suits it: the
    /// padlock over a locked one, the gear over one to open, the bag over an open one with loot in it.
    pub fn pick(&self, camera: &crate::camera::Camera, size: (u32, u32), pos: (f32, f32)) -> Option<(usize, crate::cursor::Kind)> {
        use crate::cursor::Kind;
        (0..self.chests.len())
            .filter_map(|i| {
                let c = &self.chests[i];
                let kind = match (c.state, c.locked) {
                    (State::Closed, Some(_)) => Kind::PickLock,
                    (State::Closed, None) => Kind::Interact,
                    _ if c.loot.is_some() => Kind::LootAll,
                    _ => return None,
                };
                let [x0, y0, x1, y1] = crate::creature::screen_box(camera, size, c.pos, 0.7, 1.4)?;
                (pos.0 >= x0 && pos.0 <= x1 && pos.1 >= y0 && pos.1 <= y1).then(|| ((c.pos - camera.pos).length(), i, kind))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i, kind)| (i, kind))
    }

    /// Where the chests are that can still give `item` (render space): a closed one whose fixed contents or drop table have it, or an
    /// open one with it left inside.
    pub fn holding(&self, item: usize) -> Vec<Vec3> {
        let can_give = |c: &Chest| match (c.state, &c.loot) {
            (State::Closed, _) => c.def.items.iter().any(|(id, _)| crate::items::find(id) == item) || content::drop_table(&c.def.loot).is_some_and(|t| t.items.iter().any(|d| d.0 == item)),
            (_, loot) => loot.as_ref().is_some_and(|l| l.items.iter().any(|s| s.item == item)),
        };
        self.chests.iter().filter(|c| can_give(c)).map(|c| c.pos).collect()
    }

    pub fn pos(&self, chest: usize) -> Option<Vec3> {
        self.chests.get(chest).map(|c| c.pos)
    }

    /// How hard its lock is, if it is locked.
    pub fn lock(&self, chest: usize) -> Option<Level> {
        self.chests.get(chest)?.locked
    }

    /// The lock was picked (or the chest was not locked): the lid swings up and the loot is rolled. Returns what is in the chest.
    pub fn open(&mut self, chest: usize, wanted: &[usize]) -> Option<Loot> {
        let c = self.chests.get_mut(chest)?;
        if c.state == State::Closed {
            c.locked = None;
            c.state = State::Opening(0.0);
            c.loot = contents(c.def, &mut self.rng, wanted);
        }
        c.loot.clone()
    }

    /// Every chest as it is, in the order of `chests.ron`.
    pub fn snapshot(&self) -> Vec<ChestSave> {
        self.chests.iter().map(|c| ChestSave { id: c.def.id.clone(), open: c.state != State::Closed, locked: c.locked.is_some(), loot: c.loot.as_ref().map(crate::save::loot_to_save) }).collect()
    }

    /// Put saved chests back (the n-th saved is the n-th of the content, if it is the same one). One being opened is open.
    pub fn restore(&mut self, saved: &[ChestSave]) {
        for (c, s) in self.chests.iter_mut().zip(saved).filter(|(c, s)| c.def.id == s.id) {
            c.state = if s.open { State::Open } else { State::Closed };
            c.locked = c.def.lock.filter(|_| s.locked);
            c.loot = s.loot.as_ref().map(crate::save::loot_from_save);
            // Posed again by the next update.
            c.placed = false;
        }
    }

    /// Leave `loot` in the chest (what is left after looting).
    pub fn set_loot(&mut self, chest: usize, loot: Loot) {
        if let Some(c) = self.chests.get_mut(chest) {
            c.loot = (!loot.is_empty()).then_some(loot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chest_holds_what_is_fixed_and_what_its_table_rolls() {
        let mut def = content::get().chests[0].clone();
        let mut rng = 7;
        // Only what is fixed: always the same, split into stacks when it is more than fits on one.
        (def.loot, def.copper, def.items) = (String::new(), 12, vec![("hearthstone".into(), 1), ("linen_cloth".into(), 45)]);
        let loot = contents(&def, &mut rng, &[]).unwrap();
        let cloth = crate::items::find("linen_cloth");
        assert_eq!(loot.copper, 12);
        assert_eq!(loot.items.iter().filter(|s| s.item == cloth).map(|s| s.count).collect::<Vec<_>>(), [20, 20, 5]);
        assert_eq!(loot.items.len(), 4);
        // Besides a table: at least the fixed copper on top of the table's.
        def.loot = "chest_poor".into();
        let more = contents(&def, &mut rng, &[]).unwrap();
        assert!(more.copper > 12 && more.items.len() >= 4);
        // Nothing at all is no loot.
        (def.loot, def.copper, def.items) = (String::new(), 0, Vec::new());
        assert!(contents(&def, &mut rng, &[]).is_none());
        // A quest item in it stays in only for a quest that wants it.
        def.items = vec![("tough_wolf_meat".into(), 1)];
        assert!(contents(&def, &mut rng, &[]).is_none());
        assert_eq!(contents(&def, &mut rng, &[crate::items::find("tough_wolf_meat")]).unwrap().items.len(), 1);
    }

    #[test]
    fn every_chest_of_the_content_has_a_model_with_the_poses_it_needs() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        for def in &content::get().chests {
            let m2 = M2::parse(&mount.read(&def.model).unwrap()).unwrap();
            assert!(sequence(&m2, &[STAND, CLOSED]).is_some() && sequence(&m2, &[OPENING]).is_some(), "{} lacks a pose", def.id);
        }
    }
}
