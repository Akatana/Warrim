//! Fishing: the rules of the original's server (CMaNGOS) for what a cast into water gives. A line is cast with a fishing pole in the hand at water in front; a fish
//! bites after a while and the bobber shows it for a few seconds; reeling in then catches what the water of the area holds (`classic_fishing.ron`), with a chance
//! by the skill against what the water asks for; the skill rises by a chance that falls as it grows.

use crate::items::{self, Inventory};
use crate::loot::random;

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
/// How far the line can be cast, yards.
pub const CAST_RANGE: f32 = 24.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catch_is_sure_in_water_of_the_skill_and_less_likely_in_harder_water() {
        assert_eq!([catch_chance(50, 0), catch_chance(50, -70), catch_chance(100, 100), catch_chance(300, 100)], [100; 4]);
        assert_eq!([catch_chance(50, 100), catch_chance(1, 300), catch_chance(0, 100)], [25, 1, 1]);
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
