//! What the trades make: Alchemy, Blacksmithing, Cooking, Engineering, First Aid, Leatherworking, Tailoring and the smelting of Mining (and the enchantments of
//! Enchanting, `enchants.rs`). A recipe (`content::Recipe`, from the original's spells) makes an item out of reagents, asking for tools to carry and a station
//! to stand at, and teaches the skill by how hard it is for the rank. A recipe is known when it is taught: by a trainer, or by the recipe item (a pattern, a plan, a formula)
//! used; some come with the skill.

use crate::content::{self, Recipe};
use crate::items::{self, Inventory, Stack};
use crate::loot::random;
use crate::skills::Skill;
use std::collections::BTreeSet;

/// How a recipe stands against the skill, in the names and colours of the original's window (`TradeSkillTypeColor`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Difficulty {
    /// Orange: the skill is sure to rise.
    Optimal,
    /// Yellow.
    Medium,
    /// Green.
    Easy,
    /// Grey: the skill cannot rise by it.
    Trivial,
}

impl Difficulty {
    pub fn color(self) -> [f32; 4] {
        match self {
            Difficulty::Optimal => [1.0, 0.5, 0.25, 1.0],
            Difficulty::Medium => [1.0, 1.0, 0.0, 1.0],
            Difficulty::Easy => [0.25, 0.75, 0.25, 1.0],
            Difficulty::Trivial => [0.5, 0.5, 0.5, 1.0],
        }
    }
}

pub fn difficulty(r: &Recipe, rank: u32) -> Difficulty {
    match rank {
        x if x >= r.ranks.3 => Difficulty::Trivial,
        x if x >= r.ranks.2 => Difficulty::Easy,
        x if x >= r.ranks.1 => Difficulty::Medium,
        _ => Difficulty::Optimal,
    }
}

/// The recipe of a spell of the original, with its place in the content.
pub fn get(spell: u32) -> Option<(usize, &'static Recipe)> {
    let c = content::get();
    let i = *c.recipe_numbers.get(&spell)?;
    Some((i, &c.recipes[i]))
}

/// The recipes (places in the content) of a trade, in the order the window lists them: by the kind of what they make, then by how early they are learned, then by name.
pub fn of(skill: Skill) -> Vec<usize> {
    let c = content::get();
    let mut list: Vec<usize> = (0..c.recipes.len()).filter(|&i| c.recipes[i].skill == skill).collect();
    list.sort_by(|&a, &b| (category(&c.recipes[a]), c.recipes[a].ranks.1, &c.recipes[a].name).cmp(&(category(&c.recipes[b]), c.recipes[b].ranks.1, &c.recipes[b].name)));
    list
}

/// The heading a recipe is listed under: the class of what it makes.
pub fn category(r: &Recipe) -> &'static str {
    match r.item.map(|i| items::get(i).class) {
        None => match r.worn.2 {
            1024 => "Gloves",
            1048608 => "Chest",
            512 => "Bracer",
            256 => "Boots",
            65536 => "Cloak",
            16384 => "Shield",
            _ => "Weapon",
        },
        Some(0) => "Consumable",
        Some(1) | Some(11) => "Container",
        Some(2) => "Weapon",
        Some(4) => "Armor",
        Some(5) => "Reagent",
        Some(6) => "Projectile",
        Some(7) => "Trade Goods",
        _ => "Miscellaneous",
    }
}

/// The inventory types of the original (`ItemDisplay`... the numbers `import_items.py` maps from) that an item worn at `wear` can be, as bits of a mask.
fn inventory_bits(wear: items::Wear) -> u32 {
    use items::Wear::*;
    let types: &[u32] = match wear {
        Head => &[1],
        Neck => &[2],
        Shoulder => &[3],
        Shirt => &[4],
        Chest => &[5, 20],
        Waist => &[6],
        Legs => &[7],
        Feet => &[8],
        Wrist => &[9],
        Hands => &[10],
        Finger => &[11],
        Trinket => &[12],
        OneHand => &[13],
        Shield => &[14],
        Ranged => &[15, 26],
        Back => &[16],
        TwoHand => &[17],
        Tabard => &[19],
        MainHand => &[21],
        OffHand => &[22],
        Held => &[23],
        Ammo => &[24],
        Thrown => &[25],
        None => &[],
    };
    types.iter().fold(0, |m, t| m | 1 << t)
}

/// Whether an enchantment recipe fits an item: its class, the subclasses and the inventory types it names (0: any).
pub fn fits(item: &items::ItemDef, r: &Recipe) -> bool {
    let (class, subclasses, types) = r.worn;
    (class < 0 || item.class as i32 == class) && (subclasses == 0 || subclasses >> item.subclass & 1 == 1) && (types == 0 || types & inventory_bits(item.wear) != 0)
}

/// The equipment slot an enchantment goes on: the first worn item that fits, the weapons before the armor.
pub fn target(inv: &Inventory, r: &Recipe) -> Option<usize> {
    [items::MAIN_HAND, items::OFF_HAND].into_iter().chain(0..items::MAIN_HAND).find(|&slot| inv.equipment[slot].is_some_and(|s| fits(items::get(s.item), r)))
}

/// The reagents of a recipe are used up (what an enchantment costs); false, and nothing used, if they are short.
pub fn spend(inv: &mut Inventory, r: &Recipe) -> bool {
    if makeable(inv, r) == 0 {
        return false;
    }
    r.reagents.iter().for_each(|&(item, n)| drop(inv.remove(item, n)));
    true
}

/// What the stations are called (`SpellFocusObject.dbc`).
pub fn station_name(focus: u32) -> &'static str {
    match focus {
        1 => "Anvil",
        2 => "Loom",
        3 => "Forge",
        4 => "Cooking Fire",
        663 => "Alchemy Lab",
        _ => "a special place",
    }
}

/// The recipes a character has been taught (by their spells).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Known(BTreeSet<u32>);

impl Known {
    pub fn knows(&self, r: &Recipe) -> bool {
        r.auto || self.0.contains(&r.spell)
    }

    /// Learn the recipe of `spell`: false if there is no such recipe or it was known.
    pub fn learn(&mut self, spell: u32) -> bool {
        get(spell).is_some_and(|(_, r)| !r.auto) && self.0.insert(spell)
    }

    pub fn parts(&self) -> Vec<u32> {
        self.0.iter().copied().collect()
    }

    /// Back from the save; what is no recipe (the tables changed) is dropped.
    pub fn from_parts(parts: &[u32]) -> Known {
        Known(parts.iter().copied().filter(|&s| get(s).is_some()).collect())
    }
}

/// Whether the item is carried: in the pack or worn.
pub fn carries(inv: &Inventory, item: usize) -> bool {
    inv.count(item) > 0 || inv.equipment.iter().flatten().any(|s| s.item == item)
}

/// How many times the reagents in the pack make the recipe.
pub fn makeable(inv: &Inventory, r: &Recipe) -> u32 {
    r.reagents.iter().map(|&(item, n)| inv.count(item) / n.max(1)).min().unwrap_or(u32::MAX)
}

/// The first tool that is not carried.
pub fn missing_tool(inv: &Inventory, r: &Recipe) -> Option<usize> {
    r.tools.iter().copied().find(|&t| !carries(inv, t))
}

/// Make it once: the reagents are used up and the item is in the pack. Returns the item and how many; none if there is nothing to make or the reagents are short.
pub fn make(inv: &mut Inventory, r: &Recipe, rng: &mut u32) -> Option<Stack> {
    let item = r.item?;
    if makeable(inv, r) == 0 {
        return None;
    }
    for &(reagent, n) in &r.reagents {
        inv.remove(reagent, n);
    }
    let count = r.count.0 + random(rng, r.count.1 - r.count.0 + 1);
    inv.add(Stack { item, count, enchant: 0 });
    Some(Stack { item, count, enchant: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_name(name: &str) -> &'static Recipe {
        content::get().recipes.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no recipe {name}"))
    }

    #[test]
    fn the_trades_have_their_recipes_with_items_reagents_and_stations() {
        let c = content::get();
        for skill in [Skill::Alchemy, Skill::Blacksmithing, Skill::Cooking, Skill::Enchanting, Skill::Engineering, Skill::FirstAid, Skill::Leatherworking, Skill::Mining, Skill::Tailoring] {
            assert!(!of(skill).is_empty(), "{skill:?}");
        }
        // Smelting is done at a forge with ore; a bandage at no station.
        let copper = by_name("Smelt Copper");
        assert_eq!((copper.skill, copper.focus, copper.reagents.len()), (Skill::Mining, 3, 1));
        assert_eq!(items::get(copper.item.unwrap()).name, "Copper Bar");
        assert_eq!(by_name("Linen Bandage").focus, 0);
        assert!(c.recipes.iter().filter(|r| r.item.is_some()).all(|r| !r.reagents.is_empty() || r.skill == Skill::Enchanting), "every made thing is made of something");
    }

    #[test]
    fn a_recipe_is_orange_yellow_green_then_grey_as_the_skill_rises() {
        let r = by_name("Linen Bandage");
        let (_, yellow, green, grey) = r.ranks;
        assert_eq!([difficulty(r, 1), difficulty(r, yellow), difficulty(r, green), difficulty(r, grey)], [Difficulty::Optimal, Difficulty::Medium, Difficulty::Easy, Difficulty::Trivial]);
    }

    #[test]
    fn a_recipe_is_known_when_taught_or_given_with_the_skill_and_the_save_keeps_it() {
        let taught = content::get().recipes.iter().find(|r| !r.auto).unwrap();
        let mut known = Known::default();
        assert!(!known.knows(taught));
        assert!(known.learn(taught.spell) && !known.learn(taught.spell), "once");
        assert!(known.knows(taught));
        assert!(!known.learn(1), "no such recipe");
        assert_eq!(Known::from_parts(&[taught.spell, 4_000_000_000]), known);
        if let Some(free) = content::get().recipes.iter().find(|r| r.auto) {
            assert!(Known::default().knows(free) && !Known::default().learn(free.spell));
        }
    }

    #[test]
    fn an_enchantment_goes_on_the_worn_item_that_fits_it_weapons_first() {
        let c = content::get();
        let of = |mask: u32, class: i32| c.recipes.iter().find(|r| r.skill == Skill::Enchanting && r.worn.2 == mask && r.worn.0 == class).unwrap_or_else(|| panic!("no enchantment for {mask}"));
        let (bracer, shield, weapon) = (of(512, 4), of(16384, 4), c.recipes.iter().find(|r| r.skill == Skill::Enchanting && r.worn.0 == 2 && r.worn.1 != 0).unwrap());
        let mut inv = Inventory::old_kit();
        // The starting kit wears a sword: a weapon has what to be put on, a bracer and a shield not.
        assert_eq!((target(&inv, bracer), target(&inv, shield), target(&inv, weapon)), (None, None, Some(items::MAIN_HAND)));
        let buckler = items::all().iter().position(|i| i.wear == items::Wear::Shield && i.class == 4 && i.subclass == 6).expect("a shield");
        inv.equipment[items::OFF_HAND] = Some(Stack { item: buckler, count: 1, enchant: 0 });
        assert_eq!(target(&inv, shield), Some(items::OFF_HAND));
        let wrist = items::all().iter().position(|i| i.wear == items::Wear::Wrist && i.class == 4 && i.subclass == 1).expect("a bracer of cloth");
        inv.equipment[9] = Some(Stack { item: wrist, count: 1, enchant: 0 });
        assert_eq!(target(&inv, bracer), Some(9));
        // Every enchantment of the recipes is known to the game, with a name.
        assert!(c.recipes.iter().filter(|r| r.item.is_none()).all(|r| c.enchants.get(&r.enchant).is_some_and(|e| !e.name.is_empty())));
    }

    #[test]
    fn making_it_uses_the_reagents_up_and_gives_the_item() {
        let r = by_name("Smelt Copper");
        let (ore, bar) = (r.reagents[0].0, r.item.unwrap());
        let mut inv = Inventory::new();
        let mut rng = 5;
        assert_eq!((makeable(&inv, r), make(&mut inv, r, &mut rng)), (0, None));
        inv.add(Stack { item: ore, count: 3, enchant: 0 });
        assert_eq!(makeable(&inv, r), 3 / r.reagents[0].1);
        let made = make(&mut inv, r, &mut rng).unwrap();
        assert_eq!(made.item, bar);
        assert_eq!((inv.count(ore), inv.count(bar)), (3 - r.reagents[0].1, made.count));
        // A tool is missing until it is carried.
        let tooled = content::get().recipes.iter().find(|r| !r.tools.is_empty()).unwrap();
        assert_eq!(missing_tool(&Inventory::new(), tooled), Some(tooled.tools[0]));
    }
}
