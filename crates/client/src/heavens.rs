//! The sky of the original client, on the CPU: where the sun and the moon stand over the day, how big they look, when the stars show, and the
//! meshes and vertex colours of the sky dome and the cloud dome. The numbers are WoW.exe's (the key tables are the ones built by FUN_006d3b80,
//! FUN_006ce4a0 and the like, the dome meshes FUN_006d0d10 and FUN_006d0530, the colouring FUN_006d0f50); `skygpu.rs` draws what this makes.
//!
//! Directions are in the original's world space (x north, y west, z up) until `to_render`.

use crate::renderer::Atmosphere;
use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// How far the sun and the moon are drawn from the camera; their sizes are in these units (a quad of side 1 is 4.8 degrees wide).
pub const RADIUS: f32 = 12.0;

/// A value that follows `keys` (day fraction, value) over the day, linear between them and wrapping round midnight (FUN_006cf6c0).
pub fn curve(keys: &[(f32, f32)], t: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    let n = keys.len();
    let next = keys.iter().position(|k| k.0 > t).unwrap_or(0);
    let prev = (next + n - 1) % n;
    let (a, b) = (keys[prev], keys[next]);
    let span = b.0 - a.0;
    if span.abs() < 0.001 {
        return a.1;
    }
    let span = if span < 0.0 { span + 1.0 } else { span };
    let along = t - a.0;
    let along = if along < 0.0 { along + 1.0 } else { along };
    a.1 + (b.1 - a.1) * along / span
}

fn deg(d: f32) -> f32 {
    d.to_radians()
}

/// The sun's angle from the zenith: 100 degrees (ten below the horizon) from 5:30 to ..., nearly overhead at noon.
fn sun_polar(t: f32) -> f32 {
    curve(&[(0.229167, deg(100.0)), (0.496528, deg(5.0)), (0.5, deg(5.0)), (0.503472, deg(5.0)), (0.895833, deg(100.0))], t)
}

/// The moon's: up from 22:00 to 4:00, 35 degrees from the zenith at midnight.
fn moon_polar(t: f32) -> f32 {
    curve(&[(0.0, deg(35.0)), (0.003472, deg(35.0)), (0.166667, deg(100.0)), (0.916667, deg(100.0)), (0.996528, deg(35.0))], t)
}

/// Both stay in the vertical plane of the compass bearing 45 degrees between north and west.
const BEARING: f32 = PI / 4.0;

fn direction(polar: f32) -> Vec3 {
    Vec3::new(polar.sin() * BEARING.cos(), polar.sin() * BEARING.sin(), polar.cos())
}

pub fn sun_direction(hour: f32) -> Vec3 {
    direction(sun_polar(hour / 24.0))
}

pub fn moon_direction(hour: f32) -> Vec3 {
    direction(moon_polar(hour / 24.0))
}

/// The side of the sun's disc: twice as big while it stands low.
pub fn sun_size(hour: f32) -> f32 {
    curve(&[(0.25, 2.0), (0.28125, 1.0), (0.84375, 1.0), (0.875, 2.0)], hour / 24.0)
}

/// The side of the moon's disc (the texture's scale is 1.75).
pub fn moon_size(hour: f32) -> f32 {
    curve(&[(0.041667, 1.0), (0.166667, 1.5), (0.916667, 1.5), (0.999306, 1.0)], hour / 24.0) * 1.75
}

/// How much of the glare round the sun shows: from 7:30 to 19:30, fading in and out for an hour.
pub fn sun_glare(hour: f32) -> f32 {
    curve(&[(0.270833, 0.0), (0.3125, 1.0), (0.8125, 1.0), (0.875, 0.0)], hour / 24.0)
}

/// How much of the glare round the moon shows: full from just before midnight to 2:00, gone by 3:15.
pub fn moon_glare(hour: f32) -> f32 {
    curve(&[(0.083333, 1.0), (0.135417, 0.0), (0.947917, 0.0), (0.99931, 1.0)], hour / 24.0)
}

/// How bright the stars are, 0 to 1: full from midnight to 3:00, gone by 4:30, back from 22:30.
pub fn star_light(hour: f32) -> f32 {
    curve(&[(0.125, 1.0), (0.1875, 0.0), (0.9375, 0.0), (1.0, 1.0)], hour / 24.0)
}

/// How strongly the horizon glows at sunrise and sunset: peaks at 6:30 and 21:30.
fn glow(hour: f32) -> f32 {
    curve(&[(0.125, 0.0), (0.270833, 1.0), (0.291667, 0.0), (0.854167, 0.0), (0.895833, 1.0), (0.999306, 0.0)], hour / 24.0)
}

/// How the glow spreads round the horizon, by compass bearing as a fraction of the circle: 1 under the sun, 0 at right angles, and
/// negative on the far side.
fn spread(bearing: f32) -> f32 {
    curve(&[(0.125, 1.0), (0.375, 0.0), (0.5, -0.5), (0.625, -0.7), (0.75, -0.5), (0.875, 0.0)], bearing)
}

/// Original world space (x north, y west, z up) to render space (x east, y up, z south).
pub fn to_render(v: Vec3) -> Vec3 {
    Vec3::new(-v.y, v.z, -v.x)
}

#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: Vec3,
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// A square of side `size` at `RADIUS` from the camera towards `dir` (original world space), facing the camera. Corners: top left, top right,
/// bottom left, bottom right. A corner's alpha fades out over the 0.4 units above the horizon (FUN_006d1960), so a sun that sets sinks into it.
pub fn billboard(dir: Vec3, size: f32, tint: Vec3, alpha: f32) -> [Vertex; 4] {
    let flat = dir.truncate().length();
    let right = if flat < 1e-5 { Vec3::Y } else { Vec3::new(-dir.y, dir.x, 0.0) / flat };
    let up = dir.cross(right);
    let centre = dir * RADIUS;
    let corner = |side: f32, rise: f32, uv: [f32; 2]| {
        let p = centre + (right * side + up * rise) * size * 0.5;
        let fade = if p.z < 0.4 { (p.z * 2.5).clamp(0.0, 1.0) } else { 1.0 };
        Vertex { pos: to_render(p), uv, color: [tint.x, tint.y, tint.z, alpha * fade] }
    };
    // The texture's left is the viewer's left: the original's right axis points to the viewer's left, hence the minus.
    [corner(1.0, 1.0, [0.0, 0.0]), corner(-1.0, 1.0, [1.0, 0.0]), corner(1.0, -1.0, [0.0, 1.0]), corner(-1.0, -1.0, [1.0, 1.0])]
}

/// The sky dome: an apex, five rings of 24 and the nadir, a flat cap whose horizon ring is at 0 and whose apex stands 0.29 above it.
/// Latitudes are fractions of pi from the apex (FUN_006d0d10).
const DOME_RINGS: [f32; 7] = [0.0, 0.17, 0.2, 0.23, 0.24, 0.25, 1.0];
const DOME_SEGMENTS: usize = 24;

fn dome_point(lat: f32, bearing: f32) -> Vec3 {
    let lat = lat * PI;
    Vec3::new(lat.sin() * bearing.cos(), lat.sin() * bearing.sin(), lat.cos() - (0.25 * PI).cos())
}

/// Positions of the sky dome (original world space) and the triangle list.
pub fn dome() -> (Vec<Vec3>, Vec<u16>) {
    let mut points = vec![dome_point(DOME_RINGS[0], 0.0)];
    for &lat in &DOME_RINGS[1..6] {
        points.extend((0..DOME_SEGMENTS).map(|j| dome_point(lat, TAU * j as f32 / DOME_SEGMENTS as f32)));
    }
    points.push(dome_point(DOME_RINGS[6], 0.0));
    (points, cap_indices(DOME_SEGMENTS, 5))
}

/// The triangles of a cap made of an apex, `rings` rings of `n` and an end point.
fn cap_indices(n: usize, rings: usize) -> Vec<u16> {
    let ring = |r: usize, j: usize| (1 + (r - 1) * n + j % n) as u16;
    let mut idx = Vec::new();
    for j in 0..n {
        idx.extend([0, ring(1, j), ring(1, j + 1)]);
    }
    for r in 1..rings {
        for j in 0..n {
            idx.extend([ring(r, j), ring(r + 1, j), ring(r + 1, j + 1), ring(r, j), ring(r + 1, j + 1), ring(r, j + 1)]);
        }
    }
    let end = (1 + rings * n) as u16;
    for j in 0..n {
        idx.extend([end, ring(rings, j + 1), ring(rings, j)]);
    }
    idx
}

fn mix(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a.lerp(b, t.clamp(0.0, 1.0))
}

/// The colours of the dome's vertices, in the order of `dome()`. The apex is the top colour, the rings below follow the sky bands down to
/// the fog colour at the horizon. At sunrise and sunset (where the zone's light says the sky glows) the bands keep their colour towards the
/// sun and fade to the middle band to the sides; away from the sun they darken towards the top colour (FUN_006d0f50).
pub fn dome_colors(a: &Atmosphere) -> Vec<Vec3> {
    let bands = [a.sky_top, a.sky_mid, a.sky_low[0], a.sky_low[1], a.sky_low[2], a.fog_color];
    let g = glow(a.hour) * a.highlight;
    let mut colors = vec![bands[0]];
    for ring in 1..=4 {
        let raw = bands[ring];
        let flat = mix(raw, bands[1], g);
        for j in 0..DOME_SEGMENTS {
            let h = spread(j as f32 / DOME_SEGMENTS as f32);
            colors.push(if h < 0.0 { mix(raw, bands[0], -h * g * 0.7) } else { mix(raw, flat, (1.0 - h) * g) });
        }
    }
    colors.extend(std::iter::repeat_n(bands[5], DOME_SEGMENTS + 1));
    colors
}

/// The cloud dome: the same cap, only to the horizon, 16 round and 12 rings (FUN_006d0530). Its textures are laid flat over it: a vertex's
/// coordinates are its place seen from above.
const CLOUD_RINGS: [f32; 12] = [0.0, 0.025, 0.05, 0.075, 0.1, 0.125, 0.15, 0.175, 0.205, 0.23, 0.245, 0.25];
const CLOUD_SEGMENTS: usize = 16;
/// How much of the clouds shows on each ring: they thin out towards the horizon.
const CLOUD_ALPHA: [f32; 12] = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.5, 0.0, 0.0];

pub fn cloud_dome() -> (Vec<Vertex>, Vec<u16>) {
    let vertex = |ring: usize, bearing: f32| {
        let reach = ring as f32 / (CLOUD_RINGS.len() - 1) as f32 * 0.5;
        Vertex { pos: to_render(dome_point(CLOUD_RINGS[ring], bearing)), uv: [0.5 + reach * bearing.cos(), 0.5 + reach * bearing.sin()], color: [1.0, 1.0, 1.0, CLOUD_ALPHA[ring]] }
    };
    let mut vertices = vec![vertex(0, 0.0)];
    for ring in 1..CLOUD_RINGS.len() {
        vertices.extend((0..CLOUD_SEGMENTS).map(|j| vertex(ring, TAU * j as f32 / CLOUD_SEGMENTS as f32)));
    }
    let n = CLOUD_SEGMENTS;
    let ring = |r: usize, j: usize| (1 + (r - 1) * n + j % n) as u16;
    let mut idx = Vec::new();
    for j in 0..n {
        idx.extend([0, ring(1, j), ring(1, j + 1)]);
    }
    for r in 1..CLOUD_RINGS.len() - 1 {
        for j in 0..n {
            idx.extend([ring(r, j), ring(r + 1, j), ring(r + 1, j + 1), ring(r, j), ring(r + 1, j + 1), ring(r, j + 1)]);
        }
    }
    (vertices, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_stands_overhead_at_noon_and_below_the_horizon_at_night() {
        assert!(sun_direction(12.0).z > 0.99);
        assert!(sun_direction(0.0).z < -0.1);
        // Rising at 5:30 it is ten degrees down, and it is up by 7:00.
        assert!((sun_direction(5.5).z + 10f32.to_radians().sin()).abs() < 0.01);
        assert!(sun_direction(7.0).z > 0.0);
    }

    #[test]
    fn the_moon_is_up_at_midnight_only() {
        assert!(moon_direction(0.0).z > 0.8);
        assert!(moon_direction(12.0).z < 0.0);
        assert!(moon_direction(23.0).z > 0.0);
    }

    #[test]
    fn curves_wrap_round_midnight() {
        // Stars: 1 from 22:30 rising to full at 24:00, and still full at 3:00.
        assert!((star_light(23.25) - 0.5).abs() < 0.01);
        assert_eq!(star_light(0.0), 1.0);
        assert_eq!(star_light(3.0), 1.0);
        assert_eq!(star_light(12.0), 0.0);
    }

    #[test]
    fn dome_meshes_have_a_colour_and_an_index_inside_for_every_vertex() {
        let (points, idx) = dome();
        assert_eq!(points.len(), 122);
        assert!(idx.iter().all(|&i| (i as usize) < points.len()));
        let atmosphere = Atmosphere::daytime(500.0);
        assert_eq!(dome_colors(&atmosphere).len(), points.len());
        let (clouds, idx) = cloud_dome();
        assert_eq!(clouds.len(), 1 + 11 * 16);
        assert!(idx.iter().all(|&i| (i as usize) < clouds.len()));
    }

    #[test]
    fn the_dome_ends_in_the_fog_colour_at_the_horizon_and_the_top_colour_at_the_apex() {
        let a = Atmosphere::daytime(500.0);
        let colors = dome_colors(&a);
        assert_eq!(colors[0], a.sky_top);
        assert_eq!(colors[colors.len() - 1], a.fog_color);
        // The horizon ring (index 97..121) is the fog colour all round.
        assert!(colors[97..121].iter().all(|&c| c == a.fog_color));
    }

    #[test]
    fn a_billboard_sinks_into_the_horizon() {
        let high = billboard(Vec3::new(0.0, 0.0, 1.0), 2.0, Vec3::ONE, 1.0);
        assert!(high.iter().all(|v| v.color[3] == 1.0));
        let setting = billboard(Vec3::new(1.0, 0.0, -0.1).normalize(), 2.0, Vec3::ONE, 1.0);
        assert!(setting.iter().all(|v| v.color[3] == 0.0));
    }
}
