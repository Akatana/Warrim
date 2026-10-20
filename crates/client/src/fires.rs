//! The fires of the world: the particle emitters of the models placed in it (the flames of a torch or a candelabra, a pyre's smoke, the spray of
//! a fountain, shafts of light). Only those near the camera burn, each as an effect of `effects.rs` that is lit as the camera comes near and put out
//! as it goes away or its tile is dropped.

use crate::coords::{tile_origin, TileCoord};
use crate::effects::Effects;
use crate::renderer::{Gpu, Renderer};
use crate::world::World;
use glam::{Mat4, Vec3};
use std::collections::{HashMap, HashSet};

/// Yards within which a fire burns, and how much farther it must be to be put out (so one at the edge does not flicker): a model that moves shows to the
/// original's farclip (350 yards), but only down to a speck of its glow, so it is worked out only to the shorter range of a fire's particles, which fall to a
/// quarter of their rate by 87 yards.
const RANGE: f32 = 140.0;
const LEAVE: f32 = 160.0;
/// Most that burn or move at once (the nearest).
const MOST: usize = 250;
/// Fires lit in one frame: building the first ones of a crowd takes a moment.
const LIGHTS_PER_FRAME: usize = 6;

#[derive(Default)]
pub struct Fires {
    /// The effect of each fire that burns, by its tile and its number there, and what it was made for (when the tile is made again with
    /// other props the numbers move: a fire that is not that model in that place any more is put out).
    lit: HashMap<(TileCoord, usize), (usize, Mat4, String)>,
}

impl Fires {
    /// How many burn or move now.
    pub fn burning(&self) -> usize {
        self.lit.len()
    }

    pub fn update(&mut self, world: &World, camera: Vec3, effects: &mut Effects, renderer: &mut Renderer, gpu: &Gpu) {
        let mut present = HashSet::new();
        let mut lighted = 0;
        // The nearest ones that may be lit this frame.
        let mut close: Vec<(f32, (TileCoord, usize))> = world
            .fires()
            .filter_map(|(tile, i, fire)| {
                let origin = tile_origin(tile);
                let at = Vec3::new(origin.0 as f32 + fire.matrix.w_axis.x, fire.matrix.w_axis.y, origin.1 as f32 + fire.matrix.w_axis.z);
                (at.distance(camera) < RANGE && !self.lit.contains_key(&(tile, i))).then_some((at.distance(camera), (tile, i)))
            })
            .collect();
        close.sort_by(|a, b| a.0.total_cmp(&b.0));
        let nearest: HashSet<(TileCoord, usize)> = close.into_iter().take(LIGHTS_PER_FRAME).map(|(_, k)| k).collect();
        for (tile, i, fire) in world.fires() {
            present.insert((tile, i));
            let origin = tile_origin(tile);
            let at = Mat4::from_translation(Vec3::new(origin.0 as f32, 0.0, origin.1 as f32)) * fire.matrix;
            let distance = at.w_axis.truncate().distance(camera);
            match self.lit.get(&(tile, i)) {
                None if distance < RANGE && lighted < LIGHTS_PER_FRAME && self.lit.len() < MOST && nearest.contains(&(tile, i)) => {
                    lighted += 1;
                    let effect = if fire.mesh { effects.spawn_placed(renderer, gpu, &fire.model, fire.light, at) } else { effects.spawn_emitters(renderer, gpu, &fire.model, at) };
                    if let Some(effect) = effect {
                        self.lit.insert((tile, i), (effect, fire.matrix, fire.model.clone()));
                    }
                }
                Some((effect, matrix, model)) if distance > LEAVE || *matrix != fire.matrix || *model != fire.model => {
                    effects.stop(*effect);
                    self.lit.remove(&(tile, i));
                }
                _ => {}
            }
        }
        self.lit.retain(|key, (effect, _, _)| {
            let kept = present.contains(key);
            if !kept {
                effects.stop(*effect);
            }
            kept
        });
    }
}
