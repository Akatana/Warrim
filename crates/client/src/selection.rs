//! The circle on the ground under the selected unit. The original (`FUN_006146d0` loads `Textures\UnitSelectTexture.blp` and the CVar
//! `ObjectSelectionCircle`; `FUN_00608e00` draws it for a unit as a decal on the ground under a box twice its radius tall) sizes it by the model:
//! radius = sqrt(0.5 * hypot(x extent, y extent) of the model's bounding box) * scale (`FUN_0060aee0`, 1.2 when the box is empty).
//! Shortcut: the picture is laid on a grid that follows the ground, not projected as a decal onto what stands there; the colour is by reaction
//! (as the unit names), not read from the original.

use crate::models::{BatchCpu, ModelCache, ModelCpu, ModelVertex};
use crate::renderer::{Gpu, Renderer};
use glam::{Mat4, Vec3};
use mpq::Mount;
use wowfmt::m2::Blend;

const TEXTURE: &str = r"Textures\UnitSelectTexture.blp";
/// Cells across the square the picture is laid on, so that it follows slopes.
const CELLS: usize = 8;
/// Raised off the ground so that it does not fight it for the depth buffer.
const LIFT: f32 = 0.05;
/// How far a step up or down the ground is followed from the unit's feet.
const REACH: f32 = 1.5;

/// `FUN_0060aee0`: the radius of the circle of a model with the bounds `min` and `max` (model space, z up) at `scale`.
pub fn radius(min: [f32; 3], max: [f32; 3], scale: f32) -> f32 {
    let (dx, dy) = (max[0] - min[0], max[1] - min[1]);
    if dx == 0.0 && dy == 0.0 {
        return 1.2 * scale;
    }
    (dx.hypot(dy) * 0.5).sqrt() * scale
}

pub struct SelectionDraw {
    entity: usize,
    vertices: Vec<ModelVertex>,
    shown: bool,
}

impl SelectionDraw {
    pub fn new(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu) -> Option<SelectionDraw> {
        let side = CELLS + 1;
        let vertices = vec![ModelVertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], uv: [0.0; 2], color: [255; 4] }; side * side];
        let mut indices = Vec::with_capacity(CELLS * CELLS * 6);
        for (row, col) in (0..CELLS).flat_map(|r| (0..CELLS).map(move |c| (r, c))) {
            let i = (row * side + col) as u16;
            let s = side as u16;
            indices.extend([i, i + s + 1, i + 1, i, i + s, i + s + 1]);
        }
        let cpu = ModelCpu {
            key: "selection circle".into(),
            vertices: vertices.clone(),
            batches: vec![BatchCpu { index_start: 0, index_count: indices.len() as u32, texture: Some(cache.texture(mount, TEXTURE)?), wrap: false, blend: Blend::Alpha, unlit: true, baked: Default::default(), window: false, glow: [0; 3], refs: Default::default(), particle: true }],
            indices,
            col_vertices: Vec::new(),
            col_indices: Vec::new(),
            tracks: None,
        };
        let entity = renderer.add_entity(gpu, &cpu);
        renderer.set_entity(entity, Vec3::ZERO, Mat4::IDENTITY, false);
        Some(SelectionDraw { entity, vertices, shown: false })
    }

    /// Show the circle of `radius` yards and `color` round `feet`, or hide it (`None`). `ground(x, z, max_y)` is the floor at a point.
    pub fn draw(&mut self, circle: Option<(Vec3, f32, [f32; 3])>, ground: impl Fn(f32, f32, f32) -> Option<f32>, renderer: &mut Renderer, gpu: &Gpu) {
        let Some((feet, radius, color)) = circle else {
            if self.shown {
                self.shown = false;
                renderer.set_entity(self.entity, Vec3::ZERO, Mat4::IDENTITY, false);
            }
            return;
        };
        self.shown = true;
        let side = CELLS + 1;
        let rgb = color.map(|c| (c * 255.0) as u8);
        for (i, v) in self.vertices.iter_mut().enumerate() {
            let (u, w) = ((i % side) as f32 / CELLS as f32, (i / side) as f32 / CELLS as f32);
            let (x, z) = (feet.x + (u * 2.0 - 1.0) * radius, feet.z + (w * 2.0 - 1.0) * radius);
            let y = ground(x, z, feet.y + REACH).filter(|y| (y - feet.y).abs() <= REACH).unwrap_or(feet.y);
            *v = ModelVertex { pos: [x - feet.x, y - feet.y + LIFT, z - feet.z], normal: [0.0, 1.0, 0.0], uv: [u, w], color: [rgb[0], rgb[1], rgb[2], 255] };
        }
        renderer.update_entity_vertices(gpu, self.entity, &self.vertices);
        renderer.set_entity(self.entity, feet, Mat4::IDENTITY, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_radius_is_the_root_of_half_the_diagonal_of_the_models_footprint_times_its_scale() {
        // A 3 by 4 yard footprint: the diagonal is 5, half of it 2.5.
        assert!((radius([0.0; 3], [3.0, 4.0, 2.0], 1.0) - 2.5f32.sqrt()).abs() < 1e-5);
        assert!((radius([0.0; 3], [3.0, 4.0, 2.0], 2.0) - 2.0 * 2.5f32.sqrt()).abs() < 1e-5);
        assert_eq!(radius([1.0; 3], [1.0; 3], 1.0), 1.2);
    }
}
