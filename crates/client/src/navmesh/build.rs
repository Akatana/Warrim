//! One tile of the navigation mesh, built the way MoveMapGen (`MapBuilder::buildMoveMapTile`) builds it.

use crate::movement::TriangleSource;
use glam_rerecast::{UVec3, Vec3, Vec3A};
use rerecast::{Aabb3d, AreaType, CompactHeightfield, DetailNavmesh, HeightfieldBuilder, PolygonNavmesh, TriMesh};

/// Voxel size (`BASE_UNIT_DIM`; it divides the 533.33333 yards of a map grid 2000 times) and the voxels along the side of a tile.
pub const CELL: f32 = 0.266_666_6;
const TILE_VOXELS: u16 = 80;
/// Yards along the side of a tile.
pub const TILE: f64 = CELL as f64 * TILE_VOXELS as f64;
/// The world spans 32 map grids to either side of the middle (WoW coordinates).
pub const HALF_WORLD: f64 = 32.0 * 533.333_33;
pub const BORDER: u16 = 5;
/// The border of a tile in yards.
pub const BORDER_YARDS: f64 = BORDER as f64 * CELL as f64;
// MapBuilder::getDefaultConfig, in voxels.
const WALKABLE_HEIGHT: u16 = 6;
pub const WALKABLE_CLIMB: u16 = 4;
const WALKABLE_RADIUS: u16 = 2;
const MAX_EDGE_LEN: u16 = TILE_VOXELS + 1;
const MAX_SIMPLIFICATION_ERROR: f32 = 1.8;
const MIN_REGION_AREA: u16 = 60 * 60;
const MERGE_REGION_AREA: u16 = 50 * 50;
const VERTS_PER_POLY: u16 = 6;
/// Triangles steeper than this cannot be walked at all, and those steeper than `STEEP_ANGLE` only by creatures.
const WALKABLE_ANGLE: f32 = 60.0;
const STEEP_ANGLE: f32 = 50.0;
/// `NAV_AREA_GROUND` and `NAV_AREA_GROUND_STEEP`.
pub const AREA_GROUND: u8 = 11;
pub const AREA_STEEP: u8 = 10;

/// A built tile: the polygons and their heights, in voxels from `corner` (the tile's corner with the border, WoW coordinates north and west).
pub struct Tile {
    pub corner: (f64, f64),
    pub polys: PolygonNavmesh,
    pub detail: DetailNavmesh,
}

/// The tile `(tx, ty)` (counted from the corner of the world: north, west) from the terrain and the objects of `source`; `None` if there is nothing walkable.
pub fn build(source: &impl TriangleSource, tx: i32, ty: i32) -> Option<Tile> {
    let pad = BORDER as f64 * CELL as f64;
    let corner = (-HALF_WORLD + tx as f64 * TILE - pad, -HALF_WORLD + ty as f64 * TILE - pad);
    let side = (TILE + 2.0 * pad) as f32;
    let mut found = Vec::new();
    source.gather([corner.0, corner.1, 0.0], glam::Vec3::new(0.0, 0.0, -4000.0), glam::Vec3::new(side, side, 4000.0), &mut found);
    // Recast's axes, as MoveMapGen has them: x west, y up, z north.
    let mut mesh = TriMesh::default();
    let (steep, flat) = (STEEP_ANGLE.to_radians().cos(), WALKABLE_ANGLE.to_radians().cos());
    for t in &found {
        let [a, b, c] = t.v.map(|v| Vec3A::new(v.y, v.z, v.x));
        // The triangles are double-sided: the side that looks up is the one to walk on.
        let (b, c) = if (b - a).cross(c - a).y < 0.0 { (c, b) } else { (b, c) };
        let up = (b - a).cross(c - a).normalize_or_zero().y;
        let area = if up > steep { AREA_GROUND } else if up > flat { AREA_STEEP } else { 0 };
        let first = mesh.vertices.len() as u32;
        mesh.vertices.extend([a, b, c]);
        mesh.indices.push(UVec3::new(first, first + 1, first + 2));
        mesh.area_types.push(AreaType(area));
    }
    add_liquid(source, corner, side, &found, &mut mesh);
    let high = mesh.compute_aabb()?;
    let aabb = Aabb3d { min: Vec3::new(0.0, high.min.y - 1.0, 0.0), max: Vec3::new(side, high.max.y + 1.0, side) };
    let mut field = HeightfieldBuilder { aabb, cell_size: CELL, cell_height: CELL }.build().ok()?;
    field.rasterize_triangles(&mesh, WALKABLE_CLIMB).ok()?;
    field.filter_low_hanging_walkable_obstacles(WALKABLE_CLIMB);
    field.filter_ledge_spans(WALKABLE_HEIGHT, WALKABLE_CLIMB);
    field.filter_walkable_low_height_spans(WALKABLE_HEIGHT);
    let mut compact = field.into_compact(WALKABLE_HEIGHT, WALKABLE_CLIMB).ok()?;
    compact.erode_walkable_area(WALKABLE_RADIUS);
    median_filter(&mut compact);
    compact.build_distance_field();
    compact.build_regions(BORDER, MIN_REGION_AREA, MERGE_REGION_AREA).ok()?;
    let contours = compact.build_contours(MAX_SIMPLIFICATION_ERROR, MAX_EDGE_LEN, rerecast::BuildContoursFlags::DEFAULT);
    let polys = contours.into_polygon_mesh(VERTS_PER_POLY).ok()?;
    let detail = DetailNavmesh::new(&polys, &compact, CELL * 16.0, CELL).ok()?;
    Some(Tile { corner, polys, detail })
}

/// `rcMedianFilterWalkableArea`: each walkable span takes the median area of the nine spans round it (the ones it is not joined to count as itself), so that a lone
/// span of another kind (a steep patch in a slope) goes.
fn median_filter(field: &mut CompactHeightfield) {
    let (w, h) = (field.width as i32, field.height as i32);
    let mut filtered = field.areas.clone();
    for z in 0..h {
        for x in 0..w {
            for i in field.cells[(x + z * w) as usize].index_range() {
                if !field.areas[i].is_walkable() {
                    continue;
                }
                let mut around = [field.areas[i].0; 9];
                for dir in 0..4u8 {
                    let Some(con) = field.spans[i].con(dir) else { continue };
                    let (ax, az, ai) = field.con_indices(x, z, dir, con);
                    if field.areas[ai].is_walkable() {
                        around[dir as usize * 2] = field.areas[ai].0;
                    }
                    let dir2 = (dir + 1) & 3;
                    if let Some(con2) = field.spans[ai].con(dir2) {
                        let (_, _, ai2) = field.con_indices(ax, az, dir2, con2);
                        if field.areas[ai2].is_walkable() {
                            around[dir as usize * 2 + 1] = field.areas[ai2].0;
                        }
                    }
                }
                around.sort_unstable();
                filtered[i] = AreaType(around[4]);
            }
        }
    }
    field.areas = filtered;
}

/// How much deeper than the bed the surface of a lake is where the mesh leaves it out: nobody wades there, creatures included (the original's "deep water").
const DEEP_WATER: f32 = 3.0;
/// `NAV_AREA_WATER` and `NAV_AREA_MAGMA_SLIME`.
pub const AREA_WATER: u8 = 9;
pub const AREA_MAGMA_SLIME: u8 = 8;

/// The liquid of the box as triangles of the water or magma area, laid beside the solid ones (`TerrainBuilder::loadMap`). The cells of the terrain whose surface is
/// more than `DEEP_WATER` over the lowest ground in them have neither the liquid nor the ground under the surface: the sea is not walked.
fn add_liquid(source: &impl TriangleSource, corner: (f64, f64), side: f32, solid: &[crate::movement::Tri], mesh: &mut TriMesh) {
    use std::collections::HashMap;
    let mut wet = Vec::new();
    source.gather_liquid([corner.0, corner.1, 0.0], glam::Vec3::ZERO, glam::Vec3::new(side, side, 0.0), &mut wet);
    if wet.is_empty() {
        return;
    }
    let unit = wowfmt::adt::UNIT_SIZE as f64;
    let cell_of = |north: f32, west: f32| (((corner.0 + north as f64 + HALF_WORLD) / unit).floor() as i32, ((corner.1 + west as f64 + HALF_WORLD) / unit).floor() as i32);
    // The surface and the lowest ground of each cell of the terrain with liquid.
    let mut surface: HashMap<(i32, i32), f32> = HashMap::new();
    for w in wet.iter().filter(|w| w.cell.is_some()) {
        let level = w.v.iter().map(|p| p.z).sum::<f32>() / 3.0;
        surface.entry(w.cell.expect("a cell")).and_modify(|s| *s = s.max(level)).or_insert(level);
    }
    let mut bed: HashMap<(i32, i32), f32> = HashMap::new();
    for t in solid {
        let c = (t.v[0] + t.v[1] + t.v[2]) / 3.0;
        let key = cell_of(c.x, c.y);
        if surface.contains_key(&key) && t.n.z.abs() > 0.5 {
            bed.entry(key).and_modify(|b| *b = b.min(c.z)).or_insert(c.z);
        }
    }
    let deep = |key: &(i32, i32)| surface.get(key).zip(bed.get(key)).is_some_and(|(s, b)| s - b > DEEP_WATER);
    // The solid triangles under the surface of a deep cell go (they become not walkable).
    for (i, idx) in mesh.indices.iter().enumerate() {
        // (Recast's axes: x west, y up, z north.)
        let c = (mesh.vertices[idx.x as usize] + mesh.vertices[idx.y as usize] + mesh.vertices[idx.z as usize]) / 3.0;
        let key = cell_of(c.z, c.x);
        if deep(&key) && surface.get(&key).is_some_and(|s| c.y < s - 1.0) {
            mesh.area_types[i] = AreaType(0);
        }
    }
    for w in wet.iter().filter(|w| w.cell.is_none_or(|k| !deep(&k))) {
        let [a, b, c] = w.v.map(|v| Vec3A::new(v.y, v.z, v.x));
        let (b, c) = if (b - a).cross(c - a).y < 0.0 { (c, b) } else { (b, c) };
        let first = mesh.vertices.len() as u32;
        mesh.vertices.extend([a, b, c]);
        mesh.indices.push(UVec3::new(first, first + 1, first + 2));
        mesh.area_types.push(AreaType(w.area));
    }
}
