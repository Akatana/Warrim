//! The compass at the top of the screen, as in Skyrim: a band showing the way the player faces, with the places of
//! `content/pois.ron` that are near on it, a question mark for those not discovered yet and a diamond with the name for those
//! that are. Walking close to a place discovers it.

use crate::content;
use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// How far (yards) a place shows on the compass, and how much of the horizon (radians each side of straight ahead) the band holds.
pub const RANGE: f32 = 400.0;
pub const HALF_VIEW: f32 = 80.0 * PI / 180.0;

/// The client's picture of the places of the map and the texture rectangle of cell `icon` of it (8 by 8).
pub const POI_ICONS: &str = r"Interface\Minimap\POIIcons";
pub fn poi_uv(icon: usize) -> [f32; 4] {
    let (col, row) = ((icon % 8) as f32, (icon / 8 % 8) as f32);
    [col / 8.0, row / 8.0, (col + 1.0) / 8.0, (row + 1.0) / 8.0]
}

/// The angle from where the player faces (`yaw`, clockwise from north) to the bearing `bearing`, in -PI..PI.
pub fn relative(bearing: f32, yaw: f32) -> f32 {
    (bearing - yaw + PI).rem_euclid(TAU) - PI
}

/// The bearing (clockwise from north) from `from` to `to`, render space.
fn bearing(from: Vec3, to: Vec3) -> f32 {
    (to.x - from.x).atan2(from.z - to.z)
}

pub struct Mark {
    pub poi: usize,
    /// Where it is on the band: -1 at its left end, 1 at its right end.
    pub x: f32,
    pub distance: f32,
}

fn place(i: usize) -> Vec3 {
    let at = content::get().pois[i].at;
    crate::coords::wow_to_render([at.0, at.1, 0.0])
}

/// The places within `RANGE` of `pos` that are in view when facing `yaw`.
pub fn marks(pos: Vec3, yaw: f32) -> Vec<Mark> {
    (0..content::get().pois.len())
        .filter_map(|poi| {
            let to = place(poi);
            let distance = Vec3::new(to.x - pos.x, 0.0, to.z - pos.z).length();
            let rel = relative(bearing(pos, to), yaw);
            (distance <= RANGE && rel.abs() <= HALF_VIEW).then_some(Mark { poi, x: rel / HALF_VIEW, distance })
        })
        .collect()
}

/// The nearest of the `targets` (render space) to `pos`, as seen facing `yaw`: where it is on the band (-1..1, held at the ends
/// when it is out of view), whether it is out of view, and its distance. None without targets.
pub fn nearest(pos: Vec3, yaw: f32, targets: &[Vec3]) -> Option<(f32, bool, f32)> {
    let (to, distance) = targets.iter().map(|&to| (to, Vec3::new(to.x - pos.x, 0.0, to.z - pos.z).length())).min_by(|a, b| a.1.total_cmp(&b.1))?;
    let x = relative(bearing(pos, to), yaw) / HALF_VIEW;
    Some((x.clamp(-1.0, 1.0), x.abs() > 1.0, distance))
}

/// The places not yet in `found` that the player at `pos` is close enough to discover.
pub fn discoveries(pos: Vec3, found: &[bool]) -> Vec<usize> {
    (0..found.len())
        .filter(|&i| !found[i] && Vec3::new(place(i).x - pos.x, 0.0, place(i).z - pos.z).length() <= content::get().pois[i].radius)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_to_the_north_is_ahead_when_facing_north_and_behind_when_facing_south() {
        let pos = crate::coords::wow_to_render([-8949.0, -132.0, 0.0]);
        let north = pos + Vec3::new(0.0, 0.0, -100.0);
        let rel = |yaw: f32| relative(bearing(pos, north), yaw);
        assert!(rel(0.0).abs() < 1e-5 && (rel(PI).abs() - PI).abs() < 1e-5);
        // Facing east, north is on the left; facing west, on the right.
        assert!(rel(90f32.to_radians()) < 0.0 && rel(270f32.to_radians()) > 0.0);
    }

    #[test]
    fn the_nearest_spot_is_found_and_held_at_the_end_of_the_band_when_out_of_view() {
        let pos = crate::coords::wow_to_render([-8949.0, -132.0, 0.0]);
        // 50 yards north and 300 yards north.
        let spots = [pos + Vec3::new(0.0, 0.0, -300.0), pos + Vec3::new(0.0, 0.0, -50.0)];
        let (x, out, distance) = nearest(pos, 0.0, &spots).unwrap();
        assert!(x.abs() < 1e-4 && !out && (distance - 50.0).abs() < 0.1);
        // Facing east it is on the left, out of view, held at the left end; facing west on the right.
        assert_eq!(nearest(pos, 90f32.to_radians(), &spots).map(|n| (n.0, n.1)), Some((-1.0, true)));
        assert_eq!(nearest(pos, 270f32.to_radians(), &spots).map(|n| (n.0, n.1)), Some((1.0, true)));
        assert!(nearest(pos, 0.0, &[]).is_none());
    }

    #[test]
    fn the_wolf_den_shows_when_near_and_in_view_and_is_discovered_from_close_by() {
        let den = content::get().pois.iter().position(|p| p.id == "wolf_den").unwrap();
        let at = place(den);
        let pos = at + Vec3::new(0.0, 0.0, 100.0);
        assert!(marks(pos, 0.0).iter().any(|m| m.poi == den && m.x.abs() < 1e-4), "100 yards south of it, facing north");
        assert!(marks(pos, PI).iter().all(|m| m.poi != den), "not when facing away");
        assert!(marks(pos + Vec3::new(0.0, 0.0, RANGE), 0.0).iter().all(|m| m.poi != den), "not when far");
        let mut found = vec![false; content::get().pois.len()];
        assert!(!discoveries(pos, &found).contains(&den));
        assert!(discoveries(at + Vec3::new(5.0, 0.0, 0.0), &found).contains(&den));
        found[den] = true;
        assert!(!discoveries(at, &found).contains(&den));
    }
}
