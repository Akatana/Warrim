//! Coordinate systems.
//!
//! WoW world space:  x = north, y = west, z = up.  Map tile (a, b) of `Map_a_b.adt`
//!                   covers x in ((32-b-1)*T, (32-b)*T], y in ((32-a-1)*T, (32-a)*T].
//! Render space:     X = east, Y = up, Z = south (right-handed). Tile (a, b) has its
//!                   north-west corner at ((a-32)*T, (b-32)*T) in (X, Z).

use glam::Vec3;
use wowfmt::adt::TILE_SIZE;

pub type TileCoord = (i32, i32);

pub fn wow_to_render(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

pub fn render_to_wow(v: Vec3) -> [f32; 3] {
    [-v.z, -v.x, v.y]
}

pub fn tile_of(x: f32, z: f32) -> TileCoord {
    ((32.0 + x / TILE_SIZE).floor() as i32, (32.0 + z / TILE_SIZE).floor() as i32)
}

/// Render-space position of a tile's north-west corner (f64: exact for any tile).
pub fn tile_origin(t: TileCoord) -> (f64, f64) {
    ((t.0 - 32) as f64 * TILE_SIZE as f64, (t.1 - 32) as f64 * TILE_SIZE as f64)
}
