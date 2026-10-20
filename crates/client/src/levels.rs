//! Experience and levels: how much experience each level takes, what a kill and a quest are worth, and how a level's
//! worth of growth is added up. The client has none of this (the server does it), so the numbers are the original's as
//! remembered: the table of experience per level and the kill formula are those of 1.12; the growth per level of the
//! attributes and of the base health is the game's own (a rough paladin).

/// The highest level; no experience is gained there.
pub const MAX_LEVEL: u32 = 60;

/// Experience needed to go from level `n` to `n + 1`, at index `n - 1`.
const XP_TO_NEXT: [u32; 59] = [
    400, 900, 1400, 2100, 2800, 3600, 4500, 5400, 6500, 7600, //
    8800, 10100, 11400, 12900, 14400, 16000, 17700, 19400, 21300, 23200, //
    25200, 27300, 29400, 31700, 34000, 36400, 38900, 41400, 44300, 47400, //
    50800, 54500, 58600, 62800, 67100, 71600, 76100, 80800, 85700, 90700, //
    95800, 101000, 106300, 111800, 117500, 123200, 129100, 135100, 141200, 147500, //
    153900, 160400, 167100, 173900, 180800, 187900, 195000, 202300, 209800,
];

/// Where the character stands: its level and the experience gathered towards the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub level: u32,
    pub xp: u32,
}

impl Progress {
    pub fn new() -> Progress {
        Progress { level: 1, xp: 0 }
    }

    /// Experience this level takes (0 at the highest).
    pub fn to_next(&self) -> u32 {
        XP_TO_NEXT.get(self.level as usize - 1).copied().unwrap_or(0)
    }

    pub fn at_max(&self) -> bool {
        self.level >= MAX_LEVEL
    }

    /// Add experience. The levels reached are returned in order (a big reward can cross several).
    pub fn gain(&mut self, xp: u32) -> Vec<u32> {
        let mut reached = Vec::new();
        if self.at_max() {
            return reached;
        }
        self.xp += xp;
        while !self.at_max() && self.xp >= self.to_next() {
            self.xp -= self.to_next();
            self.level += 1;
            reached.push(self.level);
        }
        if self.at_max() {
            self.xp = 0;
        }
        reached
    }
}

/// The level below which a creature is worth nothing to a character of level `player` ("grey").
fn grey_level(player: u32) -> u32 {
    match player {
        0..=5 => 0,
        6..=39 => player - player / 5 - 5,
        40..=59 => player - player / 10 - 1,
        _ => 51,
    }
}

/// The colour of a creature's level for a character of level `player` (`GetDifficultyColor`): red when it is far above him,
/// then orange, yellow for his own level, green, and grey when it is worth nothing.
pub fn difficulty_color(player: u32, mob: u32) -> [f32; 4] {
    let diff = mob as i32 - player as i32;
    match diff {
        5.. => [1.0, 0.1, 0.1, 1.0],
        3..=4 => [1.0, 0.5, 0.25, 1.0],
        -2..=2 => [1.0, 1.0, 0.0, 1.0],
        _ if mob > grey_level(player) => [0.25, 0.75, 0.25, 1.0],
        _ => [0.5, 0.5, 0.5, 1.0],
    }
}

/// Whether a creature is so far above the character that the interface shows a skull instead of its level.
pub fn is_skull(player: u32, mob: u32) -> bool {
    mob >= player + 10
}

/// The "zero difference": how many levels below him a creature is worth less and less, until nothing.
fn zero_difference(player: u32) -> u32 {
    match player {
        0..=7 => 5,
        8..=9 => 6,
        10..=11 => 7,
        12..=15 => 8,
        16..=19 => 9,
        20..=29 => 11,
        30..=39 => 12,
        40..=44 => 13,
        45..=49 => 14,
        50..=54 => 15,
        55..=59 => 16,
        _ => 17,
    }
}

/// Experience for killing a creature of level `mob` as a character of level `player`: `45 + 5 * mob`, 5% more for each
/// level the creature is above him (up to four), and less for each level below until it is grey.
pub fn kill_xp(player: u32, mob: u32) -> u32 {
    let base = 45 + 5 * mob;
    if mob >= player {
        base * (100 + 5 * (mob - player).min(4)) / 100
    } else if mob <= grey_level(player) {
        0
    } else {
        let zero = zero_difference(player);
        base * (zero - (player - mob)) / zero
    }
}

/// How much the attributes (strength, agility, stamina, intellect, spirit) grow with each level, and the base health.
const ATTRIBUTE_GROWTH: [f32; 5] = [1.6, 1.0, 1.5, 1.0, 1.1];
const HEALTH_GROWTH: u32 = 25;

/// What a level adds to the attributes of level 1.
pub fn attribute_bonus(level: u32) -> [i32; 5] {
    ATTRIBUTE_GROWTH.map(|g| (g * (level - 1) as f32) as i32)
}

/// Base health of a level (before what stamina adds).
pub fn base_health(level: u32) -> u32 {
    60 + HEALTH_GROWTH * (level - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_takes_more_than_the_one_before() {
        assert!(XP_TO_NEXT.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(Progress::new().to_next(), 400);
        assert_eq!(Progress { level: 59, xp: 0 }.to_next(), 209800);
        assert_eq!(Progress { level: MAX_LEVEL, xp: 0 }.to_next(), 0);
    }

    #[test]
    fn experience_fills_the_bar_then_levels_up_and_carries_over() {
        let mut p = Progress::new();
        assert_eq!(p.gain(399), vec![]);
        assert_eq!(p.gain(1), vec![2]);
        assert_eq!(p, Progress { level: 2, xp: 0 });
        // A big reward crosses two levels at once (900 + 1400) and keeps the rest.
        assert_eq!(p.gain(900 + 1400 + 5), vec![3, 4]);
        assert_eq!(p, Progress { level: 4, xp: 5 });
        // Nothing is gained at the top.
        let mut top = Progress { level: MAX_LEVEL, xp: 0 };
        assert_eq!(top.gain(1_000_000), vec![]);
        assert_eq!(top.xp, 0);
    }

    #[test]
    fn a_kill_is_worth_what_the_formula_says() {
        // Same level: 45 + 5 x level. Above: 5% more per level, at most four. Below: less, then grey.
        assert_eq!(kill_xp(1, 1), 50);
        assert_eq!(kill_xp(1, 3), 60 + 6);
        assert_eq!(kill_xp(1, 9), 108, "the bonus stops at four levels");
        assert_eq!(kill_xp(2, 1), 40);
        assert_eq!(kill_xp(5, 1), 10);
        assert_eq!(kill_xp(6, 1), 0, "grey");
        assert_eq!((kill_xp(20, 11), kill_xp(20, 12)), (0, 105 * 3 / 11));
        assert_eq!(kill_xp(20, 18), 135 * 9 / 11);
    }

    #[test]
    fn levels_are_coloured_by_how_hard_the_creature_is() {
        let c = |p, m| difficulty_color(p, m);
        assert_eq!(c(1, 1), [1.0, 1.0, 0.0, 1.0]);
        assert_eq!(c(1, 4), [1.0, 0.5, 0.25, 1.0]);
        assert_eq!(c(1, 6), [1.0, 0.1, 0.1, 1.0]);
        assert_eq!(c(10, 5), [0.25, 0.75, 0.25, 1.0]);
        assert_eq!(c(10, 3), [0.5, 0.5, 0.5, 1.0], "grey: nothing to gain");
        assert!(is_skull(1, 11) && !is_skull(1, 10));
    }

    #[test]
    fn growth_is_zero_at_level_one_and_rises() {
        assert_eq!(attribute_bonus(1), [0; 5]);
        assert_eq!(base_health(1), 60);
        let (a, b) = (attribute_bonus(10), attribute_bonus(20));
        assert!(a.iter().zip(b).all(|(x, y)| y > *x) && base_health(20) > base_health(10));
    }
}
