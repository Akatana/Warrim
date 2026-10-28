//! Disenchanting, as the original does it: an item whose table names a `disenchant` can be broken (the server's `DisenchantID`: a green, blue or purple
//! weapon or armor, not every one of them). The spell takes its cast time and, when it ends, the item is destroyed and what the table gives
//! is shown to loot. There is no rank of Enchanting to reach first (the original asks for none); the spell only trains the skill.

use crate::content::{self, Shatter};
use crate::items::{self, Stack};
use crate::loot::random;

/// The spell of the original (Disenchant): its stance and sound.
pub const SPELL: u32 = 13262;
/// Its cast time (3000 ms in `classic_spells.ron`).
pub const SECONDS: f32 = 3.0;
/// The ranks of Enchanting at which the spell is yellow, green and grey (its row of `SkillLineAbility.dbc`: grey 60, yellow 20): it trains the skill up to 60.
pub const RANKS: (u32, u32, u32) = (20, 40, 60);

pub fn can(item: &items::ItemDef) -> bool {
    item.disenchant != 0
}

/// What breaking `item` gives. CMaNGOS `LootGroup::Process`: the rows of group 0 are rolled one by one; of a group above 0 one row comes out: the first
/// whose share of a roll of 100 is reached (rows with a chance), and when none is, one of the rows without a chance at random.
pub fn loot(item: &items::ItemDef, rng: &mut u32) -> Vec<Stack> {
    content::get().disenchant.get(&item.disenchant).map_or_else(Vec::new, |rows| roll(rows, rng))
}

fn roll(rows: &[Shatter], rng: &mut u32) -> Vec<Stack> {
    let mut out = Vec::new();
    let mut give = |row: &Shatter, rng: &mut u32| {
        let count = (row.count.0 + random(rng, row.count.1 - row.count.0 + 1)).min(items::get(row.item).stack);
        out.push(Stack { item: row.item, count, enchant: 0 });
    };
    for row in rows.iter().filter(|r| r.group == 0) {
        if (random(rng, 100_000) as f32) / 1000.0 < row.chance {
            give(row, rng);
        }
    }
    let mut groups: Vec<u8> = rows.iter().map(|r| r.group).filter(|&g| g > 0).collect();
    groups.sort_unstable();
    groups.dedup();
    for group in groups {
        let in_group = || rows.iter().filter(move |r| r.group == group);
        let mut left = random(rng, 100_000) as f32 / 1000.0;
        let hit = in_group().filter(|r| r.chance > 0.0).find(|r| {
            left -= r.chance;
            left < 0.0
        });
        let equal: Vec<&Shatter> = in_group().filter(|r| r.chance == 0.0).collect();
        let pick = hit.or_else(|| (!equal.is_empty()).then(|| equal[random(rng, equal.len() as u32) as usize]));
        if let Some(row) = pick {
            give(row, rng);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(table: u32, item: &str, count: (u32, u32)) -> f32 {
        let (item, mut rng) = (items::find(item), 99);
        let rolls: Vec<Vec<Stack>> = (0..20_000).map(|_| roll(&content::get().disenchant[&table], &mut rng)).collect();
        assert!(rolls.iter().all(|r| r.len() == 1), "a group gives exactly one row");
        assert!(rolls.iter().flatten().filter(|s| s.item == item).all(|s| (count.0..=count.1).contains(&s.count)));
        rolls.iter().flatten().filter(|s| s.item == item).count() as f32 / rolls.len() as f32
    }

    #[test]
    fn a_green_gives_one_of_dust_essence_or_shard_by_the_servers_shares() {
        // Table 2 (armor, item level 16-20): Strange Dust 75% (2-3), Greater Magic Essence 20% (1-2), and the rest, 5%, the Small Glimmering Shard.
        assert!((share(2, "i10940", (2, 3)) - 0.75).abs() < 0.02);
        assert!((share(2, "i10939", (1, 2)) - 0.20).abs() < 0.02);
        assert!((share(2, "i10978", (1, 1)) - 0.05).abs() < 0.01);
    }

    #[test]
    fn a_blue_gives_its_shard_and_a_purple_its_crystal() {
        assert_eq!(share(41, "i10978", (1, 1)), 1.0);
        assert_eq!(share(64, "i20725", (1, 1)), 1.0);
    }

    #[test]
    fn only_what_the_server_gives_a_table_can_be_broken_and_all_of_it_gives_something() {
        let mut rng = 5;
        let broken: Vec<_> = items::all().iter().filter(|i| can(i)).collect();
        assert!(broken.len() > 6000);
        assert!(broken.iter().all(|i| matches!(i.class, 2 | 4) && (2..=4).contains(&i.quality)), "only green to purple weapons and armor");
        assert!(broken.iter().any(|i| !loot(i, &mut rng).is_empty()));
        assert!(!can(items::get(items::find("i118"))), "a potion cannot be broken");
        // The item level comes from the original's tables: a green of the table for item levels 16-20 has one in that band.
        assert!(items::all().iter().filter(|i| i.disenchant == 2).all(|i| (16..=20).contains(&i.ilvl)));
    }
}
