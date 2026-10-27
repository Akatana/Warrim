//! Using items: potions that heal at once, food, drink and bandages that restore over some seconds while the character keeps still, elixirs, scrolls
//! and oils that raise a stat for a while, and the hearthstone. What an item does is what the original's spell of it does (`ItemSpell::effects`, read from
//! `Spell.dbc` by the importer); the rest of its spell (a school's damage, a summons) is told in the tooltip only.

use crate::items::{self, Inventory, UseEffect};
use std::collections::HashMap;

/// Something that works on the character for a while: a stat raised, or health and mana restored every second while he keeps still.
#[derive(Clone, Debug, PartialEq)]
pub struct Buff {
    pub item: usize,
    /// Seconds it lasts yet.
    pub left: f32,
    /// What it adds to the strength, agility, stamina, intellect, spirit, armor and health.
    pub boosts: [i32; 7],
    /// Health and mana restored every second.
    pub regen: (f32, f32),
    /// Moving or fighting ends it (food, drink, bandages).
    pub needs_calm: bool,
}

/// A buff as saved: the item's id, seconds left, boosts, regeneration and whether it needs calm.
pub type BuffSave = (String, f32, [i32; 7], (f32, f32), bool);

/// What a spell's timed effects and shown icons are in a save (see `Consumed::timed` and `shown`).
pub type TimedSave = (Vec<(Timed, f32)>, Vec<(String, f32)>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UseError {
    /// The character has not reached the level of the item.
    Level(u32),
    /// The item is not ready yet.
    Cooldown,
    /// Food, drink and bandages cannot be used while fighting or moving.
    NotCalm,
    /// The item does nothing that the game knows.
    NothingToDo,
}

/// What a use gave at once.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Used {
    pub heal: f32,
    pub mana: f32,
    /// Stats were raised (or a boost was replaced): the stats are to be worked out again.
    pub stats_changed: bool,
    /// The hearthstone was used: the way home begins.
    pub hearth: bool,
    /// A mount item was used: the steed's display and speed bonus in percent (the game mounts, or dismounts if already riding).
    pub mount: Option<(u32, i32)>,
    /// Something to put on the weapon in the hand: the enchantment and the seconds (see `UseEffect::Temp`).
    pub temp: Option<(u32, f32)>,
}

/// What a second of time did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tick {
    pub heal: f32,
    pub mana: f32,
    pub stats_changed: bool,
    /// What a spell put on the character ran out: the stats are to be worked out again, but nothing is worn anew.
    pub timed_ended: bool,
    pub way_home: Option<WayHome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WayHome {
    /// The ten seconds are over: the character is home.
    Arrived,
    Interrupted,
}

pub const HEARTH_SECONDS: f32 = 10.0;

/// What a spell of a weapon or of the gear puts on the character for some seconds (see `procs`): an effect the stats take in, health regained every second, or a
/// shield that hurts whoever hits the character (the damage and its school).
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Timed {
    Gear(UseEffect),
    Regen(f32),
    Shield { amount: f32, school: u32 },
    /// Damage of blows that is taken up before the character is hurt, until it is used up.
    Absorb(f32),
    /// A spell (of the original) that goes off on each blow the character lands: a seal.
    Seal(u32),
    /// The character's soul is kept (a soulstone): if he dies, he gets up where he fell, with this share of his health and mana.
    Ward(f32),
    /// A spell (of the original) that goes off on whoever strikes the character (a thorns, a lightning shield, a grasp that roots).
    Struck(u32),
    /// An enchantment (a row of `SpellItemEnchantment.dbc`) put on the weapon in the hand for a time: a poison, a blessing of the weapon.
    Coat(u32),
}

#[derive(Default)]
pub struct Consumed {
    pub buffs: Vec<Buff>,
    /// What procs put on the character, and the seconds left of each.
    pub timed: Vec<(Timed, f32)>,
    cooldowns: HashMap<usize, f32>,
    /// The weapon (an item) that a lure or a stone is on: it is gone when another is wielded. None until the first look after a save is loaded.
    temp_on: Option<usize>,
    /// What the character's spells put on him, shown among the buffs: the icon (in `Interface\Icons`) and the seconds left.
    pub shown: Vec<(String, f32)>,
    /// Seconds to go before the hearthstone takes the character home.
    pub hearth: Option<f32>,
    rng: u32,
}

impl Consumed {
    /// The spell of an item that is cast when it is used, if the game knows what it does.
    fn use_spell(item: usize) -> Option<&'static items::ItemSpellDef> {
        items::get(item).spells.iter().find(|s| matches!(s.trigger, 0 | 4 | 5) && s.effects.iter().any(|e| !matches!(e, UseEffect::Boost(_, _, secs) if *secs <= 0.0)))
    }

    /// Put something on the character for `seconds`; what it was of the same kind is replaced by the new one. Returns whether the stats are to be worked out again.
    pub fn add_timed(&mut self, what: Timed, seconds: f32) -> bool {
        self.timed.retain(|(t, _)| *t != what);
        self.timed.push((what, seconds));
        matches!(what, Timed::Gear(_))
    }

    /// What a lure or a stone gives to the weapon is put on for `seconds`: a new one takes the place of an old one that gave the same (the same aura of the same kind of thing).
    /// True if the stats are to be worked out again.
    pub fn apply_temp(&mut self, weapon: Option<usize>, effects: &[UseEffect], seconds: f32) -> bool {
        self.temp_on = weapon;
        for e in effects {
            if let UseEffect::Equip(aura, misc, _) = e {
                self.timed.retain(|(t, _)| !matches!(t, Timed::Gear(UseEffect::Equip(a, m, _)) if a == aura && m == misc));
            }
            self.add_timed(Timed::Gear(*e), seconds);
        }
        !effects.is_empty()
    }

    /// The enchantment that using `item` puts on the weapon and the effects it has, if the item is a lure or a stone.
    fn temp_effects(item: usize) -> Option<Vec<UseEffect>> {
        let enchant = items::get(item).spells.iter().flat_map(|s| s.effects.iter()).find_map(|e| if let UseEffect::Temp(enchant, _) = e { Some(*enchant) } else { None })?;
        Some(crate::content::get().enchants.get(&enchant).map(|e| e.effects.clone()).unwrap_or_default())
    }

    /// The weapon in the main hand is `main` now. What was put on another weapon (a lure, a stone) is gone with it; true if so, and the stats are to be worked out again.
    pub fn weapon_changed(&mut self, main: Option<usize>) -> bool {
        if !self.buffs.iter().any(|b| Self::temp_effects(b.item).is_some()) {
            self.temp_on = None;
            return false;
        }
        // Just loaded from a save: the weapon in the hand is the one it was on.
        let on = *self.temp_on.get_or_insert(main.unwrap_or(usize::MAX));
        if Some(on) == main {
            return false;
        }
        self.drop_temps(None);
        true
    }

    /// Take off what lures and stones put on the weapon, except what `keep` (an item) put there: a weapon carries one of them.
    fn drop_temps(&mut self, keep: Option<usize>) {
        let temps: Vec<usize> = self.buffs.iter().map(|b| b.item).filter(|&i| Some(i) != keep && Self::temp_effects(i).is_some()).collect();
        self.buffs.retain(|b| !temps.contains(&b.item));
        let gone: Vec<UseEffect> = temps.iter().filter_map(|&i| Self::temp_effects(i)).flatten().collect();
        self.timed.retain(|(t, _)| !matches!(t, Timed::Gear(e) if gone.contains(e)));
        if keep.is_none() {
            self.temp_on = None;
        }
    }

    /// What the stats take in from the timed spells.
    pub fn timed_effects(&self) -> Vec<UseEffect> {
        self.timed.iter().filter_map(|(t, _)| if let Timed::Gear(e) = t { Some(*e) } else { None }).collect()
    }

    /// The shields on the character: the damage each does to whoever hits it, and its school.
    pub fn shields(&self) -> impl Iterator<Item = (f32, u32)> + '_ {
        self.timed.iter().filter_map(|(t, _)| if let Timed::Shield { amount, school } = *t { Some((amount, school)) } else { None })
    }

    /// The character died: the share of health and mana he gets up with, if his soul was kept (and it is spent).
    pub fn take_ward(&mut self) -> Option<f32> {
        let i = self.timed.iter().position(|(t, _)| matches!(t, Timed::Ward(_)))?;
        match self.timed.remove(i).0 {
            Timed::Ward(share) => Some(share),
            _ => None,
        }
    }

    /// The spells that go off on whoever strikes the character.
    pub fn reactions(&self) -> Vec<u32> {
        self.timed.iter().filter_map(|(t, _)| if let Timed::Struck(spell) = *t { Some(spell) } else { None }).collect()
    }

    /// The seals and coats that go off on a blow.
    pub fn on_blow(&self) -> Vec<Timed> {
        self.timed.iter().map(|(t, _)| *t).filter(|t| matches!(t, Timed::Seal(_) | Timed::Coat(_))).collect()
    }

    /// A blow of `amount` meets what absorbs it: returns what is left of it, and the absorption is used as far as it went.
    pub fn absorb(&mut self, mut amount: f32) -> f32 {
        for (t, _) in &mut self.timed {
            if let Timed::Absorb(left) = t {
                let taken = left.min(amount);
                (*left, amount) = (*left - taken, amount - taken);
            }
        }
        self.timed.retain(|(t, _)| !matches!(t, Timed::Absorb(left) if *left <= 0.0));
        amount
    }

    /// A number from the dice of the character's procs.
    pub fn dice(&mut self) -> u32 {
        self.rng = if self.rng == 0 { 0x2545_F491 } else { self.rng };
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// A spell puts something on the character for `seconds`: its icon is shown with the buffs (a new one of the same spell takes the place of the old).
    pub fn show(&mut self, icon: &str, seconds: f32) {
        self.shown.retain(|(i, _)| i != icon);
        self.shown.push((icon.to_string(), seconds));
    }

    /// Seconds before the item can be used again.
    pub fn cooldown(&self, item: usize) -> f32 {
        self.cooldowns.get(&item).copied().unwrap_or(0.0)
    }

    /// Whether using the item does something.
    pub fn can_use(item: usize) -> bool {
        Self::use_spell(item).is_some()
    }

    fn random(&mut self, low: i32, high: i32) -> i32 {
        self.rng = if self.rng == 0 { 0x2545_F491 } else { self.rng };
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        low + (self.rng % ((high - low).max(0) as u32 + 1)) as i32
    }

    /// Use one of the stack in backpack slot `slot`, as a character of this `level` who is `calm` (still and not fighting).
    pub fn use_item(&mut self, inv: &mut Inventory, slot: usize, level: u32, calm: bool) -> Result<Used, UseError> {
        let Some(stack) = inv.bag[slot] else { return Err(UseError::NothingToDo) };
        let def = items::get(stack.item);
        let spell = Self::use_spell(stack.item).ok_or(UseError::NothingToDo)?;
        if def.level > level {
            return Err(UseError::Level(def.level));
        }
        if self.cooldowns.get(&stack.item).is_some_and(|left| *left > 0.0) {
            return Err(UseError::Cooldown);
        }
        let slow = spell.effects.iter().any(|e| matches!(e, UseEffect::HealOver(..) | UseEffect::ManaOver(..) | UseEffect::Hearth));
        if slow && !calm {
            return Err(UseError::NotCalm);
        }
        let mut used = Used::default();
        let mut buff = Buff { item: stack.item, left: 0.0, boosts: [0; 7], regen: (0.0, 0.0), needs_calm: false };
        for effect in spell.effects {
            match *effect {
                UseEffect::Heal(low, high) => used.heal += self.random(low, high) as f32,
                UseEffect::Mana(low, high) => used.mana += self.random(low, high) as f32,
                UseEffect::HealOver(total, secs) => (buff.regen.0, buff.left, buff.needs_calm) = (buff.regen.0 + total as f32 / secs, buff.left.max(secs), true),
                UseEffect::ManaOver(total, secs) => (buff.regen.1, buff.left, buff.needs_calm) = (buff.regen.1 + total as f32 / secs, buff.left.max(secs), true),
                UseEffect::Boost(stat, amount, secs) if secs > 0.0 && (stat as usize) < 7 => (buff.boosts[stat as usize], buff.left) = (buff.boosts[stat as usize] + amount, buff.left.max(secs)),
                UseEffect::Boost(..) => {}
                UseEffect::Equip(..) | UseEffect::Teach(_) => {}
                // The buff row shows it (the item's icon and the seconds left), as the lure does on the pole.
                UseEffect::Temp(enchant, seconds) => (used.temp, buff.left) = (Some((enchant, seconds)), buff.left.max(seconds)),
                UseEffect::Hearth => used.hearth = true,
                UseEffect::Mount(display, percent) => used.mount = Some((display, percent)),
            }
        }
        if used.temp.is_some() {
            self.drop_temps(Some(stack.item));
        }
        if buff.left > 0.0 {
            // What was working of the same kind (the same item, or another food while this is food) is replaced.
            let kind = |b: &Buff| (b.regen.0 > 0.0, b.regen.1 > 0.0);
            let before = self.buffs.len();
            self.buffs.retain(|b| b.item != buff.item && !(b.needs_calm && buff.needs_calm && kind(b) == kind(&buff)));
            used.stats_changed = buff.boosts.iter().any(|b| *b != 0) || self.buffs.len() != before;
            self.buffs.push(buff);
        }
        if used.hearth {
            self.hearth = Some(HEARTH_SECONDS);
        }
        if spell.cooldown > 0 {
            self.cooldowns.insert(stack.item, spell.cooldown as f32 / 1000.0);
        }
        // One is used up by a use, unless the item has no charges to spend (the hearthstone).
        if spell.charges < 0 {
            inv.remove(stack.item, 1);
        }
        Ok(used)
    }

    /// The boosts of all that works now.
    pub fn boosts(&self) -> [i32; 7] {
        let mut sum = [0; 7];
        for b in &self.buffs {
            sum.iter_mut().zip(b.boosts).for_each(|(s, v)| *s += v);
        }
        sum
    }

    /// `dt` seconds passed. `calm`: the character stood still and did not fight.
    pub fn tick(&mut self, dt: f32, calm: bool) -> Tick {
        let mut tick = Tick::default();
        self.cooldowns.retain(|_, left| {
            *left -= dt;
            *left > 0.0
        });
        for b in &mut self.buffs {
            if b.needs_calm && !calm {
                b.left = 0.0;
            } else {
                let seconds = dt.min(b.left);
                tick.heal += b.regen.0 * seconds;
                tick.mana += b.regen.1 * seconds;
                b.left -= dt;
            }
        }
        self.shown.retain_mut(|(_, left)| {
            *left -= dt;
            *left > 0.0
        });
        for (t, left) in &mut self.timed {
            if let Timed::Regen(per_second) = *t {
                tick.heal += per_second * dt.min(*left);
            }
            *left -= dt;
        }
        tick.stats_changed = self.buffs.iter().any(|b| b.left <= 0.0 && b.boosts.iter().any(|v| *v != 0));
        tick.timed_ended = self.timed.iter().any(|(t, left)| *left <= 0.0 && matches!(t, Timed::Gear(_)));
        self.timed.retain(|(_, left)| *left > 0.0);
        self.buffs.retain(|b| b.left > 0.0);
        if let Some(left) = self.hearth {
            if !calm {
                (self.hearth, tick.way_home) = (None, Some(WayHome::Interrupted));
            } else if left - dt <= 0.0 {
                (self.hearth, tick.way_home) = (None, Some(WayHome::Arrived));
            } else {
                self.hearth = Some(left - dt);
            }
        }
        tick
    }

    /// What the character's spells put on him, for the save.
    pub fn spell_buffs(&self) -> TimedSave {
        (self.timed.clone(), self.shown.clone())
    }

    /// What works on the character now, by item ids, for the save: the buffs, the item cooldowns, the way home.
    pub fn snapshot(&self) -> (Vec<BuffSave>, Vec<(String, f32)>, Option<f32>) {
        let buffs = self.buffs.iter().map(|b| (items::get(b.item).id.to_string(), b.left, b.boosts, b.regen, b.needs_calm)).collect();
        let cooldowns = self.cooldowns.iter().map(|(&i, &left)| (items::get(i).id.to_string(), left)).collect();
        (buffs, cooldowns, self.hearth)
    }

    /// Back from a save; items the content no longer has are dropped.
    pub fn restore(&mut self, saved: &crate::save::Save) {
        let find = |id: &str| crate::content::get().items.iter().position(|i| i.id == id);
        self.buffs = saved.buffs.iter().filter_map(|(id, left, boosts, regen, needs_calm)| find(id).map(|item| Buff { item, left: *left, boosts: *boosts, regen: *regen, needs_calm: *needs_calm })).collect();
        // What a lure or a stone gave to the weapon is put on again below, for as long as its buff lasts.
        (self.timed, self.shown) = saved.spell_buffs.clone();
        self.temp_on = None;
        for (item, left) in self.buffs.iter().map(|b| (b.item, b.left)).collect::<Vec<_>>() {
            if let Some(effects) = Self::temp_effects(item) {
                self.apply_temp(None, &effects, left);
            }
        }
        self.cooldowns = saved.item_cooldowns.iter().filter_map(|(id, left)| find(id).map(|item| (item, *left))).collect();
        self.hearth = saved.hearth;
    }

    /// Forget it all (the character died, or the game began again).
    pub fn clear(&mut self) {
        *self = Consumed::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use items::{find, Stack};

    fn pack(ids: &[&str]) -> Inventory {
        let mut inv = Inventory::new();
        inv.bag = vec![None; items::BACKPACK_SIZE];
        for id in ids {
            assert!(inv.add(Stack { item: find(id), count: 3, enchant: 0 }));
        }
        inv
    }

    #[test]
    fn a_potion_heals_at_once_by_what_the_original_says_and_cannot_be_drunk_again_before_its_time() {
        let (mut inv, mut c) = (pack(&["i118"]), Consumed::default());
        let used = c.use_item(&mut inv, 0, 1, false).unwrap();
        // Minor Healing Potion: 70 to 90.
        assert!((70.0..=90.0).contains(&used.heal) && used.mana == 0.0);
        assert_eq!(inv.count(find("i118")), 2, "one is used up");
        assert_eq!(c.use_item(&mut inv, 0, 1, false), Err(UseError::Cooldown));
        c.tick(121.0, true);
        assert!(c.use_item(&mut inv, 0, 1, false).is_ok());
    }

    #[test]
    fn food_heals_over_time_only_while_the_character_keeps_still_and_is_refused_when_he_does_not() {
        let (mut inv, mut c) = (pack(&["i117", "i159"]), Consumed::default());
        assert_eq!(c.use_item(&mut inv, 0, 1, false), Err(UseError::NotCalm));
        c.use_item(&mut inv, 0, 1, true).unwrap();
        c.use_item(&mut inv, 1, 1, true).unwrap();
        // Tough Jerky: 61 health over 18 seconds, and the spring water 151 mana over the same.
        let (mut heal, mut mana) = (0.0, 0.0);
        for _ in 0..19 {
            let t = c.tick(1.0, true);
            (heal, mana) = (heal + t.heal, mana + t.mana);
        }
        assert!((heal - 61.0).abs() < 1.0 && (mana - 151.0).abs() < 1.0, "{heal} {mana}");
        assert!(c.buffs.is_empty());
        // Walking ends it.
        c.use_item(&mut inv, 0, 1, true).unwrap();
        assert_eq!(c.tick(1.0, false).heal, 0.0);
        assert!(c.buffs.is_empty());
    }

    #[test]
    fn an_elixir_raises_a_stat_for_a_while_and_asks_for_its_level() {
        let (mut inv, mut c) = (pack(&["i954", "i3825"]), Consumed::default());
        assert_eq!(c.use_item(&mut inv, 1, 10, false), Err(UseError::Level(25)));
        // Scroll of Strength: 5 strength for 30 minutes.
        c.use_item(&mut inv, 0, 10, false).unwrap();
        assert_eq!(c.boosts()[0], 5);
        let t = c.tick(1801.0, false);
        assert!(t.stats_changed && c.boosts() == [0; 7]);
    }

    #[test]
    fn buffs_item_cooldowns_and_the_way_home_survive_a_save() {
        use crate::{levels::Progress, quests::QuestLog, save::Save};
        let (mut inv, mut c) = (pack(&["i118", "i954", "hearthstone"]), Consumed::default());
        for slot in 0..3 {
            c.use_item(&mut inv, slot, 10, true).unwrap();
        }
        c.add_timed(Timed::Shield { amount: 3.0, school: 4 }, 50.0);
        c.show("Spell_Frost_FrostArmor", 50.0);
        c.tick(5.0, true);
        let mut save = Save::capture(&inv, Progress::new(), &QuestLog::default(), (0.0, 0.0), 0.0, 0.0, 1.0);
        (save.buffs, save.item_cooldowns, save.hearth) = c.snapshot();
        save.spell_buffs = c.spell_buffs();
        let save: Save = ron::from_str(&ron::to_string(&save).unwrap()).unwrap();
        let mut back = Consumed::default();
        back.restore(&save);
        assert_eq!(back.spell_buffs(), c.spell_buffs());
        assert_eq!((&back.buffs, back.hearth, back.cooldown(find("i118"))), (&c.buffs, Some(HEARTH_SECONDS - 5.0), c.cooldown(find("i118"))));
        assert!(back.cooldown(find("i118")) > 0.0 && back.boosts()[0] == 5);
    }

    #[test]
    fn a_bridle_mounts_from_level_forty_and_is_not_used_up() {
        let (mut inv, mut c) = (pack(&["i5656"]), Consumed::default());
        assert_eq!(c.use_item(&mut inv, 0, 39, true), Err(UseError::Level(40)));
        assert_eq!(c.use_item(&mut inv, 0, 40, true).unwrap().mount, Some((2404, 60)));
        assert_eq!(inv.count(find("i5656")), 3);
    }

    #[test]
    fn a_lure_raises_fishing_for_ten_minutes_and_a_better_one_takes_its_place() {
        use crate::skills::{Skill, Skills};
        let (mut inv, mut c) = (pack(&["i6529", "i6530"]), Consumed::default());
        let skills_with = |c: &Consumed| {
            let stats = items::Stats::of(&[None; items::SLOTS], 20, &[0; 7], &c.timed_effects());
            let mut s = Skills::new();
            s.set_bonus(stats.skill_bonus);
            s.bonus(Skill::Fishing)
        };
        let used = c.use_item(&mut inv, 0, 20, true).unwrap();
        let (enchant, seconds) = used.temp.expect("a lure goes on the pole");
        assert_eq!(seconds, 600.0);
        assert!(c.apply_temp(None, &crate::content::get().enchants[&enchant].effects, seconds));
        assert_eq!(skills_with(&c), 25);
        // A better lure takes the place of the first one, it does not add to it.
        let better = c.use_item(&mut inv, 1, 20, true).unwrap().temp.unwrap();
        c.apply_temp(None, &crate::content::get().enchants[&better.0].effects, better.1);
        assert_eq!(skills_with(&c), 50);
        assert_eq!(c.buffs.len(), 1, "one lure on the pole: the first one's buff is gone");
        // It runs out.
        c.tick(601.0, true);
        assert_eq!(skills_with(&c), 0);
    }

    #[test]
    fn a_lure_is_lost_with_the_pole_and_kept_by_a_save() {
        use crate::skills::Skill;
        let (mut inv, mut c) = (pack(&["i6529"]), Consumed::default());
        let (pole, other) = (Some(find("i6256")), Some(find("hearthstone")));
        let used = c.use_item(&mut inv, 0, 20, true).unwrap();
        let (enchant, seconds) = used.temp.unwrap();
        c.apply_temp(pole, &crate::content::get().enchants[&enchant].effects, seconds);
        let bonus = |c: &Consumed| {
            let mut s = crate::skills::Skills::new();
            s.set_bonus(items::Stats::of(&[None; items::SLOTS], 20, &[0; 7], &c.timed_effects()).skill_bonus);
            s.bonus(Skill::Fishing)
        };
        assert!(!c.weapon_changed(pole) && bonus(&c) == 25, "the same pole keeps it");
        // After a save and a load the pole in the hand is the one it was on.
        let (buffs, cooldowns, hearth) = c.snapshot();
        let mut loaded = Consumed::default();
        let mut save = crate::save::Save::capture(&inv, crate::levels::Progress::new(), &crate::quests::QuestLog::default(), (0.0, 0.0), 0.0, 0.0, 1.0);
        (save.buffs, save.item_cooldowns, save.hearth) = (buffs, cooldowns, hearth);
        loaded.restore(&save);
        assert_eq!(bonus(&loaded), 25);
        assert!(!loaded.weapon_changed(pole) && bonus(&loaded) == 25);
        // Another weapon in the hand: the lure is gone.
        assert!(c.weapon_changed(other));
        assert!(c.buffs.is_empty() && bonus(&c) == 0);
    }

    #[test]
    fn stones_and_lures_ask_for_their_kind_of_weapon() {
        let asks = |spell| crate::content::classic_spell(spell).and_then(|s| s.equipped).unwrap();
        let fits = |spell, subclass: u32| { let (class, mask) = asks(spell); class == 2 && mask & (1 << subclass) != 0 };
        // A sharpening stone: swords (7) but not maces (4); a weightstone the other way round; a lure only the fishing pole (20).
        assert!(fits(2828, 7) && !fits(2828, 4) && fits(3112, 4) && !fits(3112, 7));
        assert!(fits(8087, 20) && !fits(8087, 7));
    }

    #[test]
    fn the_hearthstone_takes_ten_seconds_and_moving_stops_it() {
        let (mut inv, mut c) = (pack(&["hearthstone"]), Consumed::default());
        assert!(c.use_item(&mut inv, 0, 1, true).unwrap().hearth);
        assert_eq!(inv.count(find("hearthstone")), 3, "it is not used up");
        for _ in 0..9 {
            assert_eq!(c.tick(1.0, true).way_home, None);
        }
        assert_eq!(c.tick(1.0, true).way_home, Some(WayHome::Arrived));
        c.use_item(&mut inv, 0, 1, true).unwrap();
        assert_eq!(c.tick(1.0, false).way_home, Some(WayHome::Interrupted));
    }
}
