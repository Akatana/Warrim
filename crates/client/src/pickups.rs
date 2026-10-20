//! Props that can be picked up (`pickup` of a prop in `content/props.ron`): a sack, a flower, a crate the quests ask for. They stand in the
//! world like any prop, but only while a quest being worked on still wants their item can they be used; taking one hides it, and it is
//! there again after its time (not part of the saved game: a game that is loaded has them all back).

use crate::content::{self, Prop};
use crate::models::{placement_matrix, ModelCache};
use crate::renderer::{Gpu, Renderer};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;

struct Pickup {
    item: usize,
    respawn: f32,
    /// Whether it is gone after it was taken.
    vanish: bool,
    pos: Vec3,
    orient: Mat4,
    entity: usize,
    /// Seconds until it is back, while it is gone.
    gone: Option<f32>,
    /// What was last shown (the terrain under it may not be loaded yet, and it hides while it is gone).
    shown: Option<bool>,
}

pub struct Pickups {
    list: Vec<Pickup>,
}

impl Pickups {
    /// The pickups among `props` that stand in `map` (the models that can be drawn as objects: `.m2` files).
    pub fn load(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, props: &[Prop], map: &str) -> Result<Pickups> {
        let mut list = Vec::new();
        for p in props.iter().filter(|p| p.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map))) {
            let Some(pickup) = &p.pickup else { continue };
            let model = cache.get(mount, &p.model).with_context(|| format!("pickup model {}", p.model))?;
            let entity = renderer.add_entity(gpu, &model);
            // An item that is not (yet) one of the game's (the editor is being typed in) leaves it a plain prop.
            let Some(item) = crate::items::all().iter().position(|i| i.id == pickup.item) else { continue };
            let (pos, orient) = (crate::coords::wow_to_render(p.at.into()), placement_matrix(Vec3::ZERO, p.rotation, p.scale));
            list.push(Pickup { item, respawn: pickup.respawn, vanish: pickup.vanish, pos, orient, entity, gone: None, shown: None });
        }
        Ok(Pickups { list })
    }

    /// Take them out of the world, giving their entities back.
    pub fn retire(self, renderer: &mut Renderer) {
        self.list.iter().for_each(|p| renderer.remove_entity(p.entity));
    }

    /// The time that passes for those that are gone.
    fn count_down(&mut self, dt: f32) {
        for p in &mut self.list {
            p.gone = p.gone.and_then(|left| Some(left - dt).filter(|&l| l > 0.0));
        }
    }

    /// Count the time of those that are gone, and show or hide each as it should be (hidden where the ground is not there yet).
    pub fn update(&mut self, dt: f32, ground: impl Fn(f32, f32) -> Option<f32>, renderer: &mut Renderer) {
        self.count_down(dt);
        for p in &mut self.list {
            let show = p.gone.is_none() && ground(p.pos.x, p.pos.z).is_some();
            if p.shown != Some(show) {
                p.shown = Some(show);
                renderer.set_entity(p.entity, p.pos, p.orient, show);
            }
        }
    }

    fn usable(&self, i: usize, wanted: &[usize]) -> bool {
        self.list[i].gone.is_none() && wanted.contains(&self.list[i].item)
    }

    /// The nearest one within `range` yards of `from` (feet, render space) that is there and that a quest wants (`wanted`: items).
    pub fn near(&self, from: Vec3, range: f32, wanted: &[usize]) -> Option<usize> {
        let d = |i: usize| Vec3::new(self.list[i].pos.x - from.x, 0.0, self.list[i].pos.z - from.z).length();
        (0..self.list.len()).filter(|&i| self.usable(i, wanted) && d(i) <= range).min_by(|&a, &b| d(a).total_cmp(&d(b)))
    }

    /// The one under the pointer at `pos` (pixels) on a `size` screen seen through `camera`, if it can be used.
    pub fn pick(&self, camera: &crate::camera::Camera, size: (u32, u32), pos: (f32, f32), wanted: &[usize]) -> Option<usize> {
        (0..self.list.len())
            .filter(|&i| self.usable(i, wanted))
            .filter_map(|i| {
                let [x0, y0, x1, y1] = crate::creature::screen_box(camera, size, self.list[i].pos, 0.8, 1.2)?;
                (pos.0 >= x0 && pos.0 <= x1 && pos.1 >= y0 && pos.1 <= y1).then(|| ((self.list[i].pos - camera.pos).length(), i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    }

    pub fn pos(&self, i: usize) -> Option<Vec3> {
        self.list.get(i).map(|p| p.pos)
    }

    /// Take it: it is gone until its time is up (unless it is one that stays). Returns the item (an index) it gave, or none when it was not there to take.
    pub fn take(&mut self, i: usize) -> Option<usize> {
        let p = self.list.get_mut(i).filter(|p| p.gone.is_none())?;
        p.gone = p.vanish.then_some(p.respawn);
        Some(p.item)
    }

    /// Where those are (render space) that are there and hold `item`, for the compass and the map.
    pub fn holding(&self, item: usize) -> Vec<Vec3> {
        self.list.iter().filter(|p| p.item == item && p.gone.is_none()).map(|p| p.pos).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pickup(item: usize, at: f32) -> Pickup {
        Pickup { item, respawn: 10.0, vanish: true, pos: Vec3::new(at, 0.0, 0.0), orient: Mat4::IDENTITY, entity: 0, gone: None, shown: None }
    }

    #[test]
    fn only_a_quest_that_wants_the_item_makes_it_usable_and_taking_it_hides_it_until_its_time_is_up() {
        let mut pickups = Pickups { list: vec![pickup(3, 0.0), pickup(4, 2.0), pickup(3, 30.0)] };
        let from = Vec3::new(1.0, 0.0, 0.0);
        // No quest wants anything: nothing can be used. A quest that wants item 3: the near one of them, not the far one or the other item.
        assert_eq!(pickups.near(from, 3.5, &[]), None);
        assert_eq!(pickups.near(from, 3.5, &[3]), Some(0));
        assert_eq!(pickups.near(from, 3.5, &[3, 4]), Some(0));
        assert_eq!(pickups.near(from, 0.5, &[3]), None);
        // Taken: it gives its item once, is not there to use, and the compass no longer points to it.
        assert_eq!(pickups.take(0), Some(3));
        assert_eq!((pickups.take(0), pickups.near(from, 3.5, &[3])), (None, None));
        assert_eq!(pickups.holding(3).len(), 1);
        // Its time is counted down, and it is back.
        pickups.count_down(11.0);
        assert_eq!(pickups.near(from, 3.5, &[3]), Some(0));
        // One that does not vanish stays to be used again.
        pickups.list[1].vanish = false;
        assert_eq!((pickups.take(1), pickups.take(1)), (Some(4), Some(4)));
        assert_eq!(pickups.near(from, 3.5, &[4]), Some(1));
    }
}
