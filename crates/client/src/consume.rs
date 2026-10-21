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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timed {
    Gear(UseEffect),
    Regen(f32),
    Shield { amount: f32, school: u32 },
}

#[derive(Default)]
pub struct Consumed {
    pub buffs: Vec<Buff>,
    /// What procs put on the character, and the seconds left of each (not saved).
    pub timed: Vec<(Timed, f32)>,
    cooldowns: HashMap<usize, f32>,
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

    /// What the stats take in from the timed spells.
    pub fn timed_effects(&self) -> Vec<UseEffect> {
        self.timed.iter().filter_map(|(t, _)| if let Timed::Gear(e) = t { Some(*e) } else { None }).collect()
    }

    /// The shields on the character: the damage each does to whoever hits it, and its school.
    pub fn shields(&self) -> impl Iterator<Item = (f32, u32)> + '_ {
        self.timed.iter().filter_map(|(t, _)| if let Timed::Shield { amount, school } = *t { Some((amount, school)) } else { None })
    }

    /// A number from the dice of the character's procs.
    pub fn dice(&mut self) -> u32 {
        self.rng = if self.rng == 0 { 0x2545_F491 } else { self.rng };
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
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
                UseEffect::Equip(..) => {}
                UseEffect::Hearth => used.hearth = true,
                UseEffect::Mount(display, percent) => used.mount = Some((display, percent)),
            }
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
            assert!(inv.add(Stack { item: find(id), count: 3 }));
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
        c.tick(5.0, true);
        let mut save = Save::capture(&inv, Progress::new(), &QuestLog::default(), (0.0, 0.0), 0.0, 0.0, 1.0);
        (save.buffs, save.item_cooldowns, save.hearth) = c.snapshot();
        let save: Save = ron::from_str(&ron::to_string(&save).unwrap()).unwrap();
        let mut back = Consumed::default();
        back.restore(&save);
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
