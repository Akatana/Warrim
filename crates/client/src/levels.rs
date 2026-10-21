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

/// Experience of a quest for a character of level `player`: the full `xp` (the quest's own value) up to five levels above the quest, then 80%, 60%, 40%, 20%
/// and 10% from the sixth level above it on, rounded up as CMaNGOS's `Quest::XPValue` does. A quest of level 0 gives its xp whole.
pub fn quest_xp(xp: u32, quest: u32, player: u32) -> u32 {
    let share = match player.saturating_sub(quest) {
        _ if quest == 0 => 1.0,
        0..=5 => 1.0,
        6 => 0.8,
        7 => 0.6,
        8 => 0.4,
        9 => 0.2,
        _ => 0.1,
    };
    (xp as f32 * share).ceil() as u32
}

/// A human paladin by level (index `level - 1`), from CMaNGOS's `player_classlevelstats` and `player_levelstats` (the dump in
/// tools/data/db.sql): base health, base mana, then strength, agility, stamina, intellect and spirit.
const PALADIN: [[u16; 7]; 60] = [
    [28, 59, 22, 20, 22, 20, 22],
    [26, 64, 23, 21, 23, 21, 23],
    [34, 84, 24, 21, 24, 21, 23],
    [42, 90, 25, 22, 25, 22, 24],
    [50, 112, 26, 22, 26, 22, 25],
    [58, 120, 27, 23, 27, 23, 25],
    [66, 129, 28, 23, 28, 24, 26],
    [84, 154, 29, 24, 28, 24, 26],
    [92, 165, 30, 24, 29, 25, 27],
    [100, 192, 31, 25, 30, 25, 28],
    [108, 205, 32, 25, 31, 26, 29],
    [116, 219, 33, 26, 32, 27, 29],
    [124, 249, 34, 27, 33, 27, 30],
    [132, 265, 35, 27, 34, 28, 31],
    [131, 282, 36, 28, 36, 29, 31],
    [141, 315, 38, 28, 37, 29, 32],
    [152, 334, 39, 29, 38, 30, 33],
    [164, 354, 40, 30, 39, 31, 34],
    [177, 390, 41, 30, 40, 31, 34],
    [191, 412, 42, 31, 41, 32, 35],
    [206, 435, 43, 32, 42, 33, 36],
    [222, 459, 45, 32, 43, 34, 37],
    [239, 499, 46, 33, 44, 34, 38],
    [247, 525, 47, 34, 46, 35, 38],
    [266, 552, 48, 34, 47, 36, 39],
    [286, 579, 50, 35, 48, 37, 40],
    [307, 621, 51, 36, 49, 37, 42],
    [329, 648, 52, 36, 50, 38, 43],
    [342, 675, 54, 37, 52, 39, 44],
    [366, 702, 55, 38, 53, 40, 44],
    [391, 729, 56, 39, 54, 41, 45],
    [407, 756, 58, 39, 56, 42, 46],
    [434, 798, 59, 40, 57, 42, 47],
    [462, 825, 61, 41, 58, 43, 48],
    [481, 852, 62, 42, 60, 44, 49],
    [511, 879, 64, 43, 61, 45, 50],
    [542, 906, 65, 43, 62, 46, 51],
    [564, 933, 67, 44, 64, 47, 52],
    [597, 960, 68, 45, 65, 48, 53],
    [621, 987, 70, 46, 67, 49, 54],
    [656, 1014, 71, 47, 68, 50, 55],
    [682, 1041, 73, 47, 70, 51, 56],
    [719, 1068, 74, 48, 71, 52, 57],
    [747, 1110, 76, 49, 73, 52, 58],
    [786, 1137, 78, 50, 74, 53, 59],
    [816, 1164, 79, 51, 76, 54, 60],
    [857, 1176, 81, 52, 77, 56, 61],
    [889, 1203, 83, 53, 79, 57, 63],
    [922, 1230, 84, 54, 81, 58, 65],
    [966, 1257, 86, 55, 82, 59, 66],
    [1001, 1284, 88, 56, 84, 60, 67],
    [1037, 1311, 90, 57, 86, 61, 68],
    [1084, 1338, 92, 58, 87, 62, 69],
    [1122, 1365, 93, 59, 89, 63, 70],
    [1161, 1392, 95, 60, 91, 64, 72],
    [1201, 1419, 97, 61, 93, 65, 73],
    [1252, 1446, 99, 62, 94, 66, 74],
    [1294, 1458, 101, 63, 96, 68, 75],
    [1337, 1485, 103, 64, 98, 69, 77],
    [1381, 1512, 105, 65, 100, 70, 78],
];

fn row(level: u32) -> &'static [u16; 7] {
    &PALADIN[level.clamp(1, MAX_LEVEL) as usize - 1]
}

/// Base health of a level (before what stamina adds).
pub fn base_health(level: u32) -> u32 {
    row(level)[0] as u32
}

/// Base mana of a level (before what intellect adds).
pub fn base_mana(level: u32) -> u32 {
    row(level)[1] as u32
}

/// Strength, agility, stamina, intellect and spirit of a level, before what is worn adds.
pub fn attributes(level: u32) -> [i32; 5] {
    let r = row(level);
    [r[2], r[3], r[4], r[5], r[6]].map(i32::from)
}

/// The percent chance that a blow of a weapon misses a creature of level `mob` when the character is of level `player` and the gear gives `hit` percent
/// (CMaNGOS `CalculateEffectiveMissChance` against a creature): 5, and for each point its defense is above the weapon skill (both 5 a level) 0.1 more (0.04 less
/// below), 0.6 more past two levels, never under 1 then.
pub fn melee_miss(player: u32, mob: u32, hit: f32) -> f32 {
    let diff = 5.0 * (mob as f32 - player as f32);
    let chance = 5.0 + if diff > 10.0 { 1.0 + (diff - 10.0) * 0.6 } else if diff > 0.0 { diff * 0.1 } else { diff * 0.04 };
    (chance - hit).clamp(if diff > 10.0 { 1.0 } else { 0.0 }, 100.0)
}

/// The percent chance that a spell misses a creature of level `mob` (CMaNGOS `CalculateSpellMissChance`): 4, a point for each level above the caster, 11 for each
/// past two, less the `hit` of the gear, and never under 1.
pub fn spell_miss(player: u32, mob: u32, hit: f32) -> f32 {
    let diff = mob as f32 - player as f32;
    let chance = 4.0 + if diff > 2.0 { 2.0 + (diff - 2.0) * 11.0 } else { diff };
    (chance - hit).clamp(1.0, 100.0)
}

/// How many times the numbers of a spell (written for level 1) are worth at `level`: they grow like the health of the old
/// level curve did (60 at level 1, 25 more each level), so a heal or a bolt keeps its share of the character's health.
pub fn spell_scale(level: u32) -> f32 {
    1.0 + (level.clamp(1, MAX_LEVEL) - 1) as f32 * 25.0 / 60.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spells_grow_with_the_level() {
        assert_eq!(spell_scale(1), 1.0);
        assert!(spell_scale(60) > 20.0);
    }

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
    fn what_misses_follows_the_levels() {
        assert_eq!((melee_miss(10, 10, 0.0), melee_miss(10, 8, 0.0), melee_miss(10, 12, 0.0)), (5.0, 4.6, 6.0));
        assert!((melee_miss(10, 13, 0.0) - 9.0).abs() < 1e-4 && melee_miss(10, 10, 9.0) == 0.0, "{}", melee_miss(10, 13, 0.0));
        assert_eq!((spell_miss(10, 10, 0.0), spell_miss(10, 13, 0.0), spell_miss(10, 10, 9.0)), (4.0, 17.0, 1.0));
    }

    #[test]
    fn the_table_is_the_originals_and_rises() {
        assert_eq!((base_health(1), base_mana(1), attributes(1)), (28, 59, [22, 20, 22, 20, 22]));
        assert_eq!((base_health(60), base_mana(60), attributes(60)), (1381, 1512, [105, 65, 100, 70, 78]));
        assert!(PALADIN.windows(2).all(|w| w[0][2..].iter().zip(&w[1][2..]).all(|(a, b)| b >= a)));
    }
}
