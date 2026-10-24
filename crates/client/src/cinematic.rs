//! The introduction of a race, which the original plays when a character of it is made: a camera that flies over its homeland (the path in
//! `Cameras\FlyBy<Race>.m2`, put in the world at the place and turn `CinematicCamera.dbc` gives) while a narrator tells of the race
//! (a sound entry of the same table). It is the real world seen from the camera, so this only finds the camera; `app.rs` plays it.

use crate::camera::{vertical_fov, Camera};
use crate::character::Race;
use crate::coords::wow_to_render;
use anyhow::{Context, Result};
use glam::Vec3;
use mpq::Mount;
use wowfmt::dbc::Dbc;
use wowfmt::m2::{M2Camera, M2Track, M2};

/// The row of `CinematicCamera.dbc` that is the fly-by of a race.
fn camera_id(race: Race) -> u32 {
    match race {
        Race::Human | Race::Goblin => 142,
        Race::Orc => 235,
        Race::Dwarf => 234,
        Race::NightElf => 122,
        Race::Undead => 2,
        Race::Tauren => 202,
        Race::Gnome => 162,
        Race::Troll => 182,
    }
}

/// The map the homeland of a race is on.
pub fn map_of(race: Race) -> &'static str {
    match race {
        Race::Orc | Race::Troll | Race::Tauren | Race::NightElf => "Kalimdor",
        _ => "Azeroth",
    }
}

/// The field of view of the game's camera (`DAT_008089b4`, a quarter turn), which the fly-by looks through.
const WORLD_FOV: f32 = std::f32::consts::FRAC_PI_2;

pub struct Flyby {
    /// The sound entry of the narration.
    pub sound: u32,
    /// Where the path begins in the world (WoW coordinates: north, west, up) and how the world turns it (radians).
    origin: [f32; 3],
    rotation: f32,
    camera: M2Camera,
    /// How long it flies, milliseconds.
    pub duration: f32,
    /// The model of the fly-by, for the log.
    pub model: String,
}

impl Flyby {
    pub fn load(mount: &Mount, race: Race) -> Result<Flyby> {
        let dbc = Dbc::parse(&mount.read(r"DBFilesClient\CinematicCamera.dbc")?)?;
        let row = dbc.find_by_id(camera_id(race)).with_context(|| format!("no fly-by camera for the {}", race.name()))?;
        let file = row.string(1).rsplit_once('.').map_or(row.string(1).to_string(), |(stem, _)| format!("{stem}.m2"));
        let m2 = M2::parse(&mount.read(&file)?)?;
        let camera = m2.cameras.into_iter().next().context("the fly-by model has no camera")?;
        let end = |t: &M2Track<[f32; 9]>| t.times.last().copied().unwrap_or(0) as f32;
        let duration = end(&camera.position_path).max(end(&camera.target_path));
        Ok(Flyby { sound: row.u32(2), origin: [row.f32(3), row.f32(4), row.f32(5)], rotation: row.f32(6), camera, duration, model: file })
    }

    /// A point of the camera's model in the world (render space).
    fn place(&self, p: Vec3) -> Vec3 {
        let (s, c) = self.rotation.sin_cos();
        let [north, west, up] = self.origin;
        wow_to_render([north + c * p.x - s * p.y, west + s * p.x + c * p.y, up + p.z])
    }

    /// The camera `ms` into the fly-by, for a picture of this shape. The model's own field of view is not used: the original takes the game's (the
    /// one of the world camera, ninety degrees across the corner of the picture) and only follows the path of the model's camera.
    pub fn camera_at(&self, ms: f32, aspect: f32) -> Camera {
        let (eye, target) = (self.place(path_at(&self.camera.position_path, self.camera.position_base, ms)), self.place(path_at(&self.camera.target_path, self.camera.target_base, ms)));
        let d = target - eye;
        let mut camera = Camera::new(eye, d.x.atan2(-d.z), d.y.atan2(d.x.hypot(d.z)));
        (camera.aspect, camera.fov_y) = (aspect, vertical_fov(WORLD_FOV, aspect));
        camera
    }
}

/// Where a path is at `ms`: its base plus the curve of the keys, as the original works it out (`FUN_00714260`): the two keys the time is between
/// and how far it is from the first to the second, then by the track's kind: 0 the first key, 1 a line, 2 a cubic Bezier curve whose two middle control
/// points are the "out" point of the first key and the "in" point of the second (given as places, not as directions), 3 a cubic Hermite curve with
/// those as its tangents. After the last key it stays where it is.
fn path_at(track: &M2Track<[f32; 9]>, base: [f32; 3], ms: f32) -> Vec3 {
    let (times, keys) = (&track.times, &track.keys);
    let n = times.len().min(keys.len());
    let v = |i: usize, at: usize| Vec3::new(keys[i][at], keys[i][at + 1], keys[i][at + 2]);
    let offset = match n {
        0 => Vec3::ZERO,
        _ if ms < times[0] as f32 => v(0, 0),
        _ if ms >= times[n - 1] as f32 => v(n - 1, 0),
        _ => {
            let i = (0..n - 1).rev().find(|&i| times[i] as f32 <= ms).unwrap_or(0);
            let (t0, t1) = (times[i] as f32, times[i + 1] as f32);
            let s = (ms - t0) / (t1 - t0).max(1.0);
            let (s2, s3) = (s * s, s * s * s);
            let (from, to, out, into) = (v(i, 0), v(i + 1, 0), v(i, 6), v(i + 1, 3));
            match track.interpolation {
                0 => from,
                1 => from.lerp(to, s),
                2 => {
                    let u = 1.0 - s;
                    from * (u * u * u) + out * (3.0 * s * u * u) + into * (3.0 * s2 * u) + to * s3
                }
                _ => from * (2.0 * s3 - 3.0 * s2 + 1.0) + out * (s3 - 2.0 * s2 + s) + to * (3.0 * s2 - 2.0 * s3) + into * (s3 - s2),
            }
        }
    };
    offset + Vec3::from(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_race_has_a_fly_by_that_starts_in_its_homeland_and_lasts() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        for race in Race::ALL {
            let f = Flyby::load(&mount, race).unwrap_or_else(|e| panic!("{race:?}: {e:#}"));
            assert!(f.model.to_ascii_lowercase().contains(&race.key().to_ascii_lowercase().replace("scourge", "undead")), "{race:?}: {}", f.model);
            assert!(f.duration > 10_000.0, "{race:?}: {} ms", f.duration);
            // The camera is near where the race begins (a few hundred yards) and moves along.
            let (a, b) = (f.camera_at(0.0, 16.0 / 9.0).pos, f.camera_at(f.duration / 2.0, 16.0 / 9.0).pos);
            assert!(a.distance(b) > 20.0, "{race:?} does not move");
            assert!(a.is_finite() && b.is_finite());
        }
    }

    #[test]
    fn a_path_follows_its_keys_by_the_kind_of_its_track_and_holds_at_the_ends() {
        let key = |value: [f32; 3], into: [f32; 3], out: [f32; 3]| [value, into, out].concat().try_into().unwrap();
        // From (0,0,0) to (10,0,0); the middle control points of a Bezier curve are where the tangents sit.
        let keys = vec![key([0.0; 3], [0.0; 3], [10.0, 0.0, 0.0]), key([10.0, 0.0, 0.0], [0.0, 0.0, 0.0], [10.0, 0.0, 0.0])];
        let track = |interpolation| M2Track { interpolation, global_sequence: -1, ranges: vec![], times: vec![0, 1000], keys: keys.clone() };
        let at = |kind, ms| path_at(&track(kind), [1.0, 1.0, 1.0], ms);
        for kind in 0..4 {
            assert_eq!((at(kind, -5.0), at(kind, 1000.0), at(kind, 5000.0)), (Vec3::ONE, Vec3::new(11.0, 1.0, 1.0), Vec3::new(11.0, 1.0, 1.0)));
        }
        assert_eq!(at(0, 500.0), Vec3::ONE);
        assert!((at(1, 500.0).x - 6.0).abs() < 1e-4);
        // Bezier: halfway the weights of the four points are 1/8, 3/8, 3/8, 1/8.
        assert!((at(2, 500.0).x - (1.0 + 0.375 * 10.0 + 0.125 * 10.0)).abs() < 1e-4);
        // Hermite: the value halfway is the mean of the ends plus an eighth of the tangent difference.
        assert!((at(3, 500.0).x - (1.0 + 5.0 + 0.125 * (10.0 - 0.0))).abs() < 1e-4);
    }
}
