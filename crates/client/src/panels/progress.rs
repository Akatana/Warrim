//! The character's identity, level, experience and the quests' counting of kills and items.

use super::*;

impl Panels {
    /// After a saved game was loaded into `quests`: the quests start counting from what is carried, without announcing it.
    /// Put a saved character into the windows' state: what it carries, its level, quests, skills, spells and discoveries. What the saved game no
    /// longer has under its id is dropped (see `Save::apply`).
    /// The character's name, race and sex, which the character sheet, the quest texts and the tooltips of items go by.
    pub fn set_identity(&mut self, name: &str, race: crate::character::Race, sex: crate::character::Sex) {
        (self.name, self.race, self.sex) = (name.to_string(), race, sex);
        content::set_horde(race.horde());
    }

    /// A new character puts on what the original gave its race and sex.
    pub fn give_outfit(&mut self) {
        self.inv = Inventory::outfitted(self.race, self.sex);
        (self.equipment_changed, self.stats_dirty) = (true, true);
    }

    pub fn restore(&mut self, saved: &crate::save::Save) {
        for note in saved.apply(&mut self.inv, &mut self.progress, &mut self.quests) {
            log::warn!("saved game: {note}");
        }
        self.spent = saved.spent;
        self.talents = crate::talents::Ranks::from_saved(&saved.talents);
        self.pet = saved.pet.clone();
        self.pet_spells = saved.pet_spells.clone();
        self.sync_points();
        self.found = content::get().pois.iter().map(|p| saved.discovered.contains(&p.id)).collect();
        self.explored = saved.explored.clone();
        if !saved.skills.is_empty() {
            self.skills = crate::skills::Skills::from_parts(&saved.skills);
            self.skills.set_steps(&saved.professions);
        }
        self.recipes = crate::recipes::Known::from_parts(&saved.recipes);
        if !saved.spells.is_empty() {
            self.known = saved.spells.iter().filter_map(|id| content::get().spells.iter().position(|s| s.id == *id)).collect();
        }
        for (id, left) in &saved.cooldowns {
            let at = content::get().spells.iter().position(|s| s.id == *id);
            if let Some(c) = at.and_then(|i| self.cooldowns.get_mut(i)) {
                *c = *left;
            }
        }
        // Before the refresh: the stats count the boosts.
        self.consumed.restore(saved);
        self.refresh();
        self.quests_restored();
    }

    /// The content was replaced: what the windows show of creatures, loot and quests is of the old one, so it is closed and forgotten.
    pub fn forget_world(&mut self) {
        self.close_all();
        self.close_popup();
        self.set_target(None);
        (self.loot, self.closed_loot, self.buyback, self.gossip, self.merchant, self.questing, self.reward_choice) = (None, None, None, None, None, None, None);
        self.log_selected = 0;
    }

    pub fn quests_restored(&mut self) {
        self.seen.clear();
        self.ready.clear();
        for quest in self.quests.active.iter().map(|a| a.quest).collect::<Vec<_>>() {
            self.refresh_quest_marks(quest);
        }
    }

    // ---- messages and progress

    /// `format("%s: %d/%d")` of a global string.
    pub(super) fn counted(&self, key: &str, name: &str, have: u32, need: u32) -> String {
        self.string(key).replacen("%s", name, 1).replacen("%d", &have.to_string(), 1).replacen("%d", &need.to_string(), 1)
    }

    /// Creatures of these kinds were killed: they give experience, and the kill goals of the quests move on, with a
    /// message each.
    pub fn on_kills(&mut self, kinds: &[usize]) {
        for &kind in kinds {
            let creature = &content::get().creatures[kind];
            let xp = levels::kill_xp(self.progress.level, creature.level);
            if xp > 0 {
                self.notice(&self.string("COMBATLOG_XPGAIN_FIRSTPERSON").replacen("%s", &creature.name, 1).replacen("%d", &xp.to_string(), 1));
                self.gain_xp(xp);
            }
            for step in self.quests.killed(kind) {
                self.notice(&self.counted("ERR_QUEST_ADD_KILL_SII", &creature.name, step.have, step.need));
            }
        }
        self.update_quests();
    }

    /// The level is set outright (the console): the stats follow, and the effect plays if it rose.
    pub fn set_level(&mut self, level: u32) {
        let rose = level > self.progress.level;
        self.progress = Progress { level, xp: 0 };
        self.sync_points();
        self.refresh();
        self.equipment_changed = true;
        if rose {
            self.events.push(Sfx::Named("LEVELUPSOUND"));
            self.level_ups += 1;
        }
    }

    /// Add experience; reaching a level says so, and the stats (health most of all) grow with it.
    pub fn gain_xp(&mut self, xp: u32) {
        let health = self.stats.max_health;
        let reached = self.progress.gain(xp);
        if reached.is_empty() {
            return;
        }
        self.refresh();
        // The player takes on the new stats (health) when the equipment is applied again.
        self.equipment_changed = true;
        self.sync_points();
        for level in reached {
            self.notice(&self.string("LEVEL_UP").replacen("%d", &level.to_string(), 1));
            self.events.push(Sfx::Named("LEVELUPSOUND"));
            self.level_ups += 1;
            if level >= crate::talents::FIRST_LEVEL {
                self.notice("You have a new talent point. Press N to spend it.");
            }
        }
        let gained = (self.stats.max_health - health).round() as i32;
        if gained > 0 {
            self.notice(&self.string("LEVEL_UP_HEALTH").replacen("%d", &gained.to_string(), 1));
        }
    }

    /// Call every frame: says when an item a quest wants was picked up, and when a quest can be turned in.
    pub fn update_quests(&mut self) {
        self.fill_log();
        let active: Vec<usize> = self.quests.active.iter().map(|a| a.quest).collect();
        for quest in active {
            for (g, goal) in content::quest(quest).goals.iter().enumerate() {
                if let Goal::Collect { item, count } = *goal {
                    let have = self.quests.have(quest, g, &self.inv);
                    let before = self.seen.insert((quest, g), have).unwrap_or(have);
                    if have > before {
                        self.notice(&self.counted("ERR_QUEST_ADD_ITEM_SII", crate::items::get(item).name, have, count));
                    }
                }
            }
            let done = self.quests.complete(quest, &self.inv);
            if done && self.ready.insert(quest) {
                self.notice(&self.string("ERR_QUEST_COMPLETE_S").replacen("%s", &content::quest(quest).title, 1));
                self.events.push(Sfx::Named("QUESTCOMPLETED"));
            } else if !done {
                self.ready.remove(&quest);
            }
        }
    }

    /// Whether a level was gained since the last call.
    pub fn take_level_up(&mut self) -> bool {
        std::mem::take(&mut self.level_ups) > 0
    }

    /// What a creature of kind `def` shows over its head for the quests.
    pub fn marker(&self, kind: usize) -> Option<Marker> {
        self.quests.marker(kind, &self.inv, self.who())
    }

    /// Who the quests are for: the character's level and race.
    pub fn who(&self) -> crate::quests::Who {
        crate::quests::Who { level: self.progress.level, race: 1 << self.race as u32 }
    }

    /// The character is at (north, west) of `map`: a quest that asks to walk there moves on.
    pub fn arrive(&mut self, map: &str, north: f32, west: f32) {
        let steps = self.quests.arrived(map, north, west);
        for step in &steps {
            self.notice(&format!("{}: Destination reached", content::quest(step.quest).title));
        }
        if !steps.is_empty() {
            self.update_quests();
        }
    }
}
