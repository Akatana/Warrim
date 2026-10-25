//! Nodes standing in the world (`content/classic_nodes.ron`): the mineral veins and herbs that Mining and Herbalism gather. A node is drawn only while
//! the viewer is near (there are thousands); gathering it takes a cast, rolls its drop table and counts one of its uses, and a node whose uses are
//! spent is gone until its time is up (those that are gone are in the saved game, with the time left).

use crate::content::{self, NodeKind, NODE_RESPAWN};
use crate::loot::{random, Loot};
use crate::models::{placement_matrix, ModelCache};
use crate::renderer::{Gpu, Renderer};
use glam::{Mat4, Vec3};
use mpq::Mount;

/// How far from the viewer a node gets its model, and how far it keeps it, yards.
const SHOW_RANGE: f32 = 140.0;
const KEEP_RANGE: f32 = 170.0;
/// How many models are made in one frame at most.
const MADE_PER_FRAME: usize = 2;

struct Node {
    kind: usize,
    pos: Vec3,
    orient: Mat4,
    respawn: f32,
    /// How many times it can still be gathered.
    uses: u32,
    /// What a gathering left in it, to take.
    loot: Option<Loot>,
    entity: Option<usize>,
    /// Seconds until it is back, while it is gone.
    gone: Option<f32>,
}

pub struct Nodes {
    list: Vec<Node>,
    cache: ModelCache,
    rng: u32,
}

fn kind(n: &Node) -> &'static NodeKind {
    &content::get().node_kinds[n.kind]
}

impl Nodes {
    /// The nodes of the content that stand in `map`.
    pub fn load(map: &str) -> Nodes {
        let c = content::get();
        let mut rng = 0x0DE5_1A7E;
        let list = c
            .nodes
            .iter()
            .filter(|n| n.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map)))
            .filter_map(|n| {
                let k = *c.node_numbers.get(&n.kind)?;
                let def = &c.node_kinds[k];
                let uses = def.uses.0 + random(&mut rng, def.uses.1 - def.uses.0 + 1);
                let orient = placement_matrix(Vec3::ZERO, [0.0, n.facing, 0.0], def.scale);
                Some(Node { kind: k, pos: crate::coords::wow_to_render(n.at.into()), orient, respawn: n.respawn.unwrap_or(NODE_RESPAWN), uses, loot: None, entity: None, gone: None })
            })
            .collect();
        Nodes { list, cache: ModelCache::default(), rng }
    }

    /// Take them out of the world, giving their entities back.
    pub fn retire(self, renderer: &mut Renderer) {
        self.list.iter().filter_map(|n| n.entity).for_each(|e| renderer.remove_entity(e));
    }

    /// A number from 0 to 1.
    pub fn roll(&mut self) -> f32 {
        random(&mut self.rng, 10_000) as f32 / 10_000.0
    }

    /// The time that passes for those that are gone; one that is back can be gathered as many times as a new one.
    fn count_down(&mut self, dt: f32) {
        for i in 0..self.list.len() {
            let Some(left) = self.list[i].gone else { continue };
            if left > dt {
                self.list[i].gone = Some(left - dt);
            } else {
                let def = kind(&self.list[i]);
                self.list[i].uses = def.uses.0 + random(&mut self.rng, def.uses.1 - def.uses.0 + 1);
                self.list[i].gone = None;
            }
        }
    }

    /// Count the time of those that are gone, and give a model to those that came near and take it from those that went far or were gathered empty.
    pub fn update(&mut self, dt: f32, viewer: Vec3, mount: &Mount, gpu: &Gpu, renderer: &mut Renderer) {
        self.count_down(dt);
        let mut made = 0;
        for n in &mut self.list {
            let far = (n.pos.x - viewer.x).hypot(n.pos.z - viewer.z);
            match n.entity {
                Some(e) if n.gone.is_some() || far > KEEP_RANGE => {
                    renderer.remove_entity(e);
                    n.entity = None;
                }
                None if n.gone.is_none() && far <= SHOW_RANGE && made < MADE_PER_FRAME => {
                    made += 1;
                    if let Some(model) = self.cache.get(mount, &kind(n).model) {
                        let e = renderer.add_entity(gpu, &model);
                        renderer.set_entity(e, n.pos, n.orient, true);
                        n.entity = Some(e);
                    }
                }
                _ => {}
            }
        }
    }

    /// Those that are gone as saved: (their place in the list, seconds until they are back).
    pub fn snapshot(&self) -> Vec<(usize, f32)> {
        self.list.iter().enumerate().filter_map(|(i, n)| Some((i, n.gone?))).collect()
    }

    pub fn restore(&mut self, saved: &[(usize, f32)]) {
        for &(i, left) in saved {
            if let Some(n) = self.list.get_mut(i) {
                n.gone = Some(left.max(0.0)).filter(|&l| l > 0.0);
            }
        }
    }

    /// Whether there is something to do with it: it is there, and it can be gathered or has loot left.
    fn usable(&self, i: usize) -> bool {
        let n = &self.list[i];
        n.gone.is_none() && (n.uses > 0 || n.loot.is_some())
    }

    /// The nearest one within `range` yards of `from` (feet, render space) that can be used.
    pub fn near(&self, from: Vec3, range: f32) -> Option<usize> {
        let d = |i: usize| Vec3::new(self.list[i].pos.x - from.x, 0.0, self.list[i].pos.z - from.z).length();
        (0..self.list.len()).filter(|&i| self.usable(i) && d(i) <= range).min_by(|&a, &b| d(a).total_cmp(&d(b)))
    }

    /// The nodes that can be gathered within the compass's range of `pos` that are in view when facing `yaw`: where each is on the band (-1 at its left end, 1
    /// at its right), how far it is, the profession that gathers it and its name.
    pub fn marks(&self, pos: Vec3, yaw: f32) -> Vec<(f32, f32, crate::skills::Skill, &'static str)> {
        use crate::compass::{bearing, relative, HALF_VIEW, RANGE};
        (0..self.list.len())
            .filter(|&i| self.usable(i))
            .filter_map(|i| {
                let n = &self.list[i];
                let distance = Vec3::new(n.pos.x - pos.x, 0.0, n.pos.z - pos.z).length();
                let rel = relative(bearing(pos, n.pos), yaw);
                (distance <= RANGE && rel.abs() <= HALF_VIEW).then(|| (rel / HALF_VIEW, distance, kind(n).skill, kind(n).name.as_str()))
            })
            .collect()
    }

    /// The one under the pointer at `pos` (pixels) on a `size` screen seen through `camera`, if it can be used.
    pub fn pick(&self, camera: &crate::camera::Camera, size: (u32, u32), pos: (f32, f32)) -> Option<usize> {
        (0..self.list.len())
            .filter(|&i| self.usable(i) && self.list[i].entity.is_some())
            .filter_map(|i| {
                let [x0, y0, x1, y1] = crate::creature::screen_box(camera, size, self.list[i].pos, 0.8, 1.2)?;
                (pos.0 >= x0 && pos.0 <= x1 && pos.1 >= y0 && pos.1 <= y1).then(|| ((self.list[i].pos - camera.pos).length(), i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    }

    pub fn pos(&self, i: usize) -> Option<Vec3> {
        self.list.get(i).map(|n| n.pos)
    }

    pub fn kind(&self, i: usize) -> Option<&'static NodeKind> {
        self.list.get(i).map(kind)
    }

    /// What a gathering left in it that was not taken.
    pub fn loot(&self, i: usize) -> Option<&Loot> {
        self.list.get(i)?.loot.as_ref()
    }

    /// Gather it: one of its uses is spent and its table is rolled (quest items only for a quest that wants them: `wanted`). Returns what it gave,
    /// none when there was nothing to give or the node was not there to gather.
    pub fn gather(&mut self, i: usize, wanted: &[usize]) -> Option<Loot> {
        if !self.usable(i) || self.list[i].uses == 0 {
            return None;
        }
        let table = content::drop_table(&kind(&self.list[i]).loot);
        let mut loot = table.map(|t| t.roll(&mut self.rng)).unwrap_or_default();
        loot.only_wanted(wanted);
        self.list[i].uses -= 1;
        self.set_loot(i, loot.clone());
        Some(loot).filter(|l| !l.is_empty())
    }

    /// Leave `loot` in the node (what is left after looting); a node with no uses and nothing left is gone until its time is up.
    pub fn set_loot(&mut self, i: usize, loot: Loot) {
        if let Some(n) = self.list.get_mut(i) {
            n.loot = (!loot.is_empty()).then_some(loot);
            if n.uses == 0 && n.loot.is_none() {
                n.gone = Some(n.respawn);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes_of(kind_id: &str, count: usize) -> Nodes {
        let k = content::get().node_numbers[kind_id];
        let list = (0..count).map(|i| Node { kind: k, pos: Vec3::new(i as f32 * 10.0, 0.0, 0.0), orient: Mat4::IDENTITY, respawn: 10.0, uses: 2, loot: None, entity: None, gone: None }).collect();
        Nodes { list, cache: ModelCache::default(), rng: 7 }
    }

    #[test]
    fn the_content_has_veins_and_herbs_and_every_node_stands_in_a_map_of_the_game() {
        let c = content::get();
        assert!(c.node_kinds.iter().any(|k| k.skill == crate::skills::Skill::Mining && k.rank == 0), "copper");
        assert!(c.node_kinds.iter().any(|k| k.skill == crate::skills::Skill::Herbalism && k.rank >= 290), "black lotus and the like");
        assert!(c.nodes.len() > 1000 && Nodes::load("Azeroth").list.len() > 1000);
        // Every kind has the table of its gathering, and a mining kind asks for a rank of mining.
        assert!(c.node_kinds.iter().all(|k| content::drop_table(&k.loot).is_some_and(|t| !t.items.is_empty())));
    }

    #[test]
    fn the_compass_shows_the_nodes_in_range_and_in_view_that_are_there() {
        let mut nodes = nodes_of("n1731", 3);
        // Nodes at x = 0, 10 and 20 (render space); standing 100 yards south of the first, facing north.
        let pos = Vec3::new(0.0, 0.0, 100.0);
        assert_eq!(nodes.marks(pos, 0.0).len(), 3);
        assert!(nodes.marks(pos, std::f32::consts::PI).is_empty(), "not behind");
        assert!(nodes.marks(pos + Vec3::new(0.0, 0.0, crate::compass::RANGE), 0.0).is_empty(), "not beyond the range");
        nodes.list[1].gone = Some(5.0);
        assert_eq!(nodes.marks(pos, 0.0).len(), 2, "not one that is gone");
    }

    #[test]
    fn every_kind_of_node_has_a_model_that_loads() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let cache = ModelCache::default();
        for k in &content::get().node_kinds {
            assert!(cache.get(&mount, &k.model).is_some_and(|m| !m.vertices.is_empty()), "{}: {}", k.id, k.model);
        }
    }

    #[test]
    fn a_node_is_gathered_as_often_as_it_has_uses_then_it_is_gone_until_its_time_is_up() {
        let mut nodes = nodes_of("n1731", 2);
        let here = Vec3::new(1.0, 0.0, 0.0);
        assert_eq!(nodes.near(here, 3.5), Some(0));
        // A copper vein always gives copper ore (it is the table's first, sure item).
        let ore = nodes.gather(0, &[]).expect("ore");
        assert!(ore.items.iter().any(|s| crate::items::get(s.item).name == "Copper Ore"));
        // Something left in it can still be taken, and it is not gone while it holds loot.
        assert!(nodes.loot(0).is_some() && nodes.list[0].uses == 1);
        nodes.set_loot(0, Loot::default());
        assert!(nodes.near(here, 3.5) == Some(0) && nodes.gather(0, &[]).is_some());
        // The last use is spent: with nothing left to take it is gone, and back after its time.
        nodes.set_loot(0, Loot::default());
        assert_eq!((nodes.near(here, 3.5), nodes.gather(0, &[])), (None, None));
        assert_eq!(nodes.snapshot(), vec![(0, 10.0)]);
        let mut again = nodes_of("n1731", 2);
        again.restore(&nodes.snapshot());
        assert_eq!(again.list[0].gone, Some(10.0));
        nodes.count_down(11.0);
        assert_eq!(nodes.near(here, 3.5), Some(0));
        assert!(nodes.list[0].uses >= 1);
    }
}
