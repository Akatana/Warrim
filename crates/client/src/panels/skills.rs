//! Skills, spells and enchantments the character knows.

use super::*;

impl Panels {
    /// The skill was used `uses` times; a new rank says so.
    pub fn train(&mut self, skill: crate::skills::Skill, uses: f32) {
        if let Some(rank) = self.skills.train(skill, uses) {
            self.notice(&format!("Your skill in {} has increased to {rank}.", skill.name()));
            if skill.group() == crate::skills::Group::Armor {
                self.refresh();
            }
        }
        self.fill_skills();
    }

    /// What the enchantments of the worn items give, as effects of worn spells.
    pub(super) fn enchant_effects(&self) -> Vec<crate::items::UseEffect> {
        self.inv.equipment.iter().flatten().filter_map(|s| content::get().enchants.get(&s.enchant)).flat_map(|e| e.effects.iter().copied()).collect()
    }

    /// Put an enchantment on what is worn in `slot` (what was on it before is gone); it goes with the item.
    pub fn enchant_slot(&mut self, slot: usize, enchant: u32) {
        if let Some(stack) = &mut self.inv.equipment[slot] {
            stack.enchant = enchant;
            self.stats_dirty = true;
            self.refresh();
        }
    }

    /// Whether the trade is learned; if it is not, the screen says so.
    pub fn must_know(&mut self, skill: crate::skills::Skill) -> bool {
        let knows = self.skills.knows(skill);
        if !knows {
            self.error(&format!("You have not learned {}. A teacher in a town can teach it.", skill.name()));
        }
        knows
    }

    /// A gathering of something that asks for rank `required` of a profession: the profession may rise (`roll` from 0 to 1 against the chance).
    pub fn gather_skill(&mut self, skill: crate::skills::Skill, required: u32, roll: f32) {
        self.skill_up(skill, (required + 25, required + 50, required + 100), roll);
    }

    /// The skill may rise with this chance (0 to 1; `roll` from 0 to 1).
    pub fn raise_skill(&mut self, skill: crate::skills::Skill, chance: f32, roll: f32) {
        if let Some(rank) = self.skills.raise(skill, chance, roll) {
            self.notice(&format!("Your skill in {} has increased to {rank}.", skill.name()));
            self.fill_skills();
        }
    }

    /// Work of a trade that is yellow, green and grey from these ranks: the skill may rise (`roll` from 0 to 1 against the chance).
    pub fn skill_up(&mut self, skill: crate::skills::Skill, thresholds: (u32, u32, u32), roll: f32) {
        if let Some(rank) = self.skills.skill_up(skill, thresholds, roll) {
            self.notice(&format!("Your skill in {} has increased to {rank}.", skill.name()));
            self.fill_skills();
        }
    }

    /// Whether the item with this id is carried, in the pack or worn.
    pub fn carries(&self, id: &str) -> bool {
        crate::items::all().iter().position(|i| i.id == id).is_some_and(|item| self.inv.count(item) > 0 || self.inv.equipment.iter().flatten().any(|s| s.item == item))
    }

    /// The skill a swing of the sword trains: that of the weapon in the main hand, or the fists.
    /// The skill of the weapon in the ranged slot.
    pub fn ranged_skill(&self) -> crate::skills::Skill {
        self.inv.equipment[crate::items::RANGED].map_or(crate::skills::Skill::Unarmed, |w| crate::skills::Skill::of_weapon(crate::items::get(w.item).subclass))
    }

    /// The skill of the weapon in the main hand, or in the off hand.
    pub fn weapon_skill(&self, off: bool) -> crate::skills::Skill {
        self.inv.equipment[if off { crate::items::OFF_HAND } else { crate::items::MAIN_HAND }].map_or(crate::skills::Skill::Unarmed, |w| crate::skills::Skill::of_weapon(crate::items::get(w.item).subclass))
    }

    /// The spell is known from now on and goes on the action bar.
    pub fn learn(&mut self, spell: usize) -> bool {
        if self.known.contains(&spell) {
            return false;
        }
        self.known.push(spell);
        self.learned.push(spell);
        true
    }

    /// The spells learned since the last call.
    pub fn take_learned(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.learned)
    }

    pub fn tick_cooldowns(&mut self, dt: f32) {
        self.cooldowns.iter_mut().for_each(|c| *c = (*c - dt).max(0.0));
    }
}
