//! What a killed creature drops: some copper and a few items, rolled from its drop table when it dies.

use crate::items::{self, Stack};

/// The contents of a corpse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loot {
    pub copper: u32,
    pub items: Vec<Stack>,
}

impl Loot {
    pub fn is_empty(&self) -> bool {
        self.copper == 0 && self.items.is_empty()
    }

    /// Take out the quest items nobody wants: a quest item drops only while an active quest still needs it (`wanted`, from
    /// `QuestLog::wanted`).
    pub fn only_wanted(&mut self, wanted: &[usize]) {
        self.items.retain(|s| !is_quest_item(s.item) || wanted.contains(&s.item));
    }
}

/// Whether this item is a quest item.
pub fn is_quest_item(item: usize) -> bool {
    items::get(item).quest
}

/// What a kind of creature can drop (`content/loot.ron`): a copper range, and items (by index) with a count range and
/// a chance in percent.
#[derive(Debug)]
pub struct DropTable {
    pub copper: (u32, u32),
    pub items: Vec<(usize, (u32, u32), f32)>,
}

/// A step of a small random number generator (xorshift), returning a number below `below`.
pub fn random(rng: &mut u32, below: u32) -> u32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    *rng % below.max(1)
}

impl DropTable {
    pub fn roll(&self, rng: &mut u32) -> Loot {
        let copper = self.copper.0 + random(rng, self.copper.1 - self.copper.0 + 1);
        let mut items = Vec::new();
        for &(item, (low, high), percent) in &self.items {
            if (random(rng, 100_000) as f32) / 1000.0 < percent {
                let count = (low + random(rng, high - low + 1)).min(items::get(item).stack);
                items.push(Stack { item, count });
            }
        }
        Loot { copper, items }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quest_items_drop_only_for_a_quest_that_wants_them() {
        let (meat, cloth) = (items::find("tough_wolf_meat"), items::find("linen_cloth"));
        assert!(is_quest_item(meat) && !is_quest_item(cloth));
        let loot = Loot { copper: 3, items: vec![Stack { item: meat, count: 2 }, Stack { item: cloth, count: 1 }] };
        let (mut none, mut wanted) = (loot.clone(), loot.clone());
        none.only_wanted(&[]);
        wanted.only_wanted(&[meat]);
        assert_eq!((none.items, wanted.items, wanted.copper), (vec![Stack { item: cloth, count: 1 }], loot.items, 3));
    }

    #[test]
    fn a_wolf_drops_the_items_of_the_originals_young_wolf_by_their_chances() {
        let mut rng = 12345;
        let wolf = crate::content::drop_table("wolf").unwrap();
        let rolls: Vec<Loot> = (0..2000).map(|_| wolf.roll(&mut rng)).collect();
        // The animals of the original carry no money.
        assert!(rolls.iter().all(|l| l.copper == 0));
        let chance = |item: usize| rolls.iter().filter(|l| l.items.iter().any(|s| s.item == item)).count() as f32 / rolls.len() as f32;
        assert!((chance(items::find("tough_wolf_meat")) - 0.80).abs() < 0.05, "meat {}", chance(items::find("tough_wolf_meat")));
        assert!((chance(items::find("ruined_pelt")) - 0.37).abs() < 0.05);
        assert!((chance(items::find("i7073")) - 0.38).abs() < 0.05, "a broken fang");
        // Counts stay in their ranges.
        assert!(rolls.iter().flat_map(|l| &l.items).all(|s| (1..=2).contains(&s.count)));
        assert!(Loot::default().is_empty() && !Loot { copper: 1, items: Vec::new() }.is_empty());
    }
}
