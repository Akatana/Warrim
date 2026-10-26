//! Fishing: the rules of the original's server (CMaNGOS) for what a cast into water gives. A line is cast with a fishing pole in the hand at water in front; a fish
//! bites after a while and the bobber shows it for a few seconds; reeling in then catches what the water of the area holds (`classic_fishing.ron`), with a chance
//! by the skill against what the water asks for; the skill rises by a chance that falls as it grows.

use crate::items::{self, Inventory};
use crate::loot::random;
use glam::Vec3;

/// Whether a fishing pole is in the hand (a weapon of the subclass fishing pole of the original's tables).
pub fn pole_in_hand(inv: &Inventory) -> bool {
    inv.equipment[items::MAIN_HAND].is_some_and(|s| {
        let item = items::get(s.item);
        item.class == 2 && item.subclass == 20
    })
}

/// The chance in percent that a bite is a catch: all of it when the skill is what the water asks for, else the square of how near it is to it, a hundredth at least.
pub fn catch_chance(skill: u32, water_skill: i32) -> u32 {
    if water_skill <= 0 || skill as i32 >= water_skill {
        return 100;
    }
    (((skill as f64 / water_skill as f64).powi(2) * 100.0) as u32).max(1)
}

/// The chance in percent that a catch raises the skill: sure up to 75, then less.
pub fn rise_chance(skill: u32) -> u32 {
    if skill < 75 { 100 } else { 2500 / (skill - 50) }
}

/// Seconds from the cast until a fish bites, and how long the bobber shows it (the line goes on for twenty seconds).
pub fn bite_after(rng: &mut u32) -> f32 {
    4.0 + random(rng, 1200) as f32 / 100.0
}

pub const BITE_SECONDS: f32 = 3.0;
pub const LINE_SECONDS: f32 = 20.0;
/// The range of the fishing spell (range 8 of `SpellRange.dbc`), yards: the bobber lands between these, and the line tries this many spots before it fails.
pub const MIN_CAST: f32 = 10.0;
pub const MAX_CAST: f32 = 20.0;
pub const CAST_TRIES: u32 = 10;
/// The body radius the original adds to the distance and to the spread, yards.
const BODY: f32 = 0.5;

/// The line from the pole to the bobber is 65 points (`FUN_0061f780`), and its middle hangs half a yard below the straight way.
pub const LINE_POINTS: usize = 65;
const LINE_SAG: f32 = 0.5;
/// How far above the water the line is tied to the bobber, yards: half the height of the model (the original takes half the height of the object).
pub const BOBBER_TOP: f32 = 0.4;

/// The points of the line from `from` (the tip of the pole) to `to` (the bobber), as the original draws it: evenly spaced, each sagging by `sin(pi t)` of the sag.
pub fn line_points(from: Vec3, to: Vec3) -> [Vec3; LINE_POINTS] {
    std::array::from_fn(|i| {
        let t = i as f32 / (LINE_POINTS - 1) as f32;
        from.lerp(to, t) - Vec3::Y * LINE_SAG * (t * std::f32::consts::PI).sin()
    })
}

/// A spot for the bobber as the original's server picks it: a distance in the range of the spell, and an angle off where the caster faces that is the wider the
/// wider the range is against the distance (so the area is about as wide as it is deep). Returns (yards, radians).
pub fn cast_roll(rng: &mut u32) -> (f32, f32) {
    let mut unit = || random(rng, 10_000) as f32 / 10_000.0;
    let distance = MIN_CAST + unit() * (MAX_CAST - MIN_CAST) + BODY;
    let spread = (MAX_CAST - MIN_CAST) / (MAX_CAST + BODY);
    (distance, spread * (unit() - 0.5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catch_is_sure_in_water_of_the_skill_and_less_likely_in_harder_water() {
        assert_eq!([catch_chance(50, 0), catch_chance(50, -70), catch_chance(100, 100), catch_chance(300, 100)], [100; 4]);
        assert_eq!([catch_chance(50, 100), catch_chance(1, 300), catch_chance(0, 100)], [25, 1, 1]);
    }

    #[test]
    fn the_bobber_lands_within_the_range_of_the_spell_and_a_cone_of_its_spread() {
        let mut rng = 9;
        let rolls: Vec<_> = (0..500).map(|_| cast_roll(&mut rng)).collect();
        assert!(rolls.iter().all(|&(d, a)| (MIN_CAST + BODY..=MAX_CAST + BODY).contains(&d) && a.abs() <= 0.25));
        // It is not always the same spot.
        assert!(rolls.iter().any(|r| r.0 > 17.0) && rolls.iter().any(|r| r.0 < 13.0) && rolls.iter().any(|r| r.1 > 0.1) && rolls.iter().any(|r| r.1 < -0.1));
    }

    #[test]
    fn the_line_runs_from_the_pole_to_the_bobber_and_hangs_half_a_yard_in_the_middle() {
        let (a, b) = (Vec3::new(0.0, 2.0, 0.0), Vec3::new(10.0, 0.4, 0.0));
        let p = line_points(a, b);
        assert!(p[0].distance(a) < 1e-4 && p[64].distance(b) < 1e-4);
        assert!((p[32] - (a + b) / 2.0 + Vec3::Y * 0.5).length() < 1e-4, "{:?}", p[32]);
        assert!(p.iter().all(|q| q.y <= a.y.max(b.y)));
    }

    #[test]
    fn the_skill_rises_less_and_less_often() {
        assert_eq!([rise_chance(1), rise_chance(74), rise_chance(75), rise_chance(150), rise_chance(300)], [100, 100, 100, 25, 10]);
    }

    #[test]
    fn a_fish_bites_within_the_time_of_the_line_and_water_has_fish_where_the_game_has_tables() {
        let mut rng = 7;
        assert!((0..200).all(|_| (4.0..16.0).contains(&bite_after(&mut rng))));
        let c = crate::content::get();
        assert!(c.fishing.len() > 500 && c.fishing.values().all(|w| crate::content::drop_table(&w.table).is_some_and(|t| !t.items.is_empty())));
        // The pole of the original is a two-handed weapon of the subclass.
        let pole = items::find("i6256");
        assert_eq!((items::get(pole).class, items::get(pole).subclass), (2, 20));
    }
}
