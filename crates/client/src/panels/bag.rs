//! The pack: tomes, using items, mounting, timed effects and the messages they raise.

use super::*;

impl Panels {
    /// Read the tome in backpack slot `slot`: it teaches its spell if the class skill is high enough, and is used up.
    pub fn read_tome(&mut self, slot: usize) {
        let Some(stack) = self.inv.bag[slot] else { return };
        let Some(spell) = crate::items::get(stack.item).tome else { return };
        let def = content::spell(spell);
        if self.known.contains(&spell) {
            return self.error("You already know that spell.");
        }
        if self.skills.rank(def.class) < def.level {
            self.events.push(Sfx::Named("HumanMale_CantLearnSpell"));
            return self.error(&format!("Requires {} ({}).", def.class.name(), def.level));
        }
        self.inv.remove(stack.item, 1);
        self.learn(spell);
        self.events.push(Sfx::Named("igSpellBookSpellIconPickup"));
        self.notice(&format!("You have learned a new spell: {}.", def.name));
    }

    /// The tomes that a kill may drop: those of spells that are not known yet, whose class skill is high enough, and that are not in the pack already.
    pub fn learnable_tomes(&self) -> Vec<usize> {
        static TOMES: std::sync::OnceLock<Vec<usize>> = std::sync::OnceLock::new();
        let all = TOMES.get_or_init(|| content::get().items.iter().enumerate().filter(|(_, it)| it.tome.is_some()).map(|(i, _)| i).collect());
        all.iter()
            .copied()
            .filter(|&item| {
                let spell = crate::items::get(item).tome.unwrap_or_default();
                let def = content::spell(spell);
                !self.known.contains(&spell) && self.skills.rank(def.class) >= def.level && !self.inv.bag.iter().flatten().any(|s| s.item == item)
            })
            .collect()
    }

    /// Whether the mouse carries an item (which is drawn instead of the pointer).
    pub fn carrying(&self) -> bool {
        self.inv.cursor.is_some()
    }

    /// An item carried to the world (not onto a window or a button) is to be destroyed: ask first. False when nothing is carried.
    pub fn ask_destroy(&mut self) -> bool {
        let Some(c) = self.inv.cursor.filter(|_| !self.popup_open()) else { return false };
        self.destroying = true;
        let question = self.string("DELETE_ITEM").replacen("%s", crate::items::get(c.item).name, 1);
        self.tree.set_text("StaticPopup1Text", &question);
        self.tree.set_text("StaticPopup1Button1Text", &self.string("YES"));
        self.tree.set_text("StaticPopup1Button2Text", &self.string("NO"));
        self.tree.show(sleep::SLIDER, false);
        self.tree.show(POPUP, true);
        true
    }

    /// The answer "yes": what the mouse carries is gone.
    pub(super) fn destroy_asked(&mut self) {
        self.destroying = false;
        self.inv.cursor = None;
        self.tree.show(POPUP, false);
        self.events.push(Sfx::Named("igBackPackClose"));
        self.refresh();
        self.after_close();
    }

    /// The pointer for what is under `pos` (pixels) when it is over a window: the bag over a ware to buy, the gauntlet over
    /// the rest of the window; `None` when it is over no window.
    pub fn cursor_at(&mut self, pos: (f32, f32), screen: (u32, u32)) -> Option<crate::cursor::Kind> {
        use crate::cursor::Kind;
        if !self.any_open() {
            return None;
        }
        let hit = self.tree.hit_test(pos, screen)?;
        let name = self.tree.name(hit);
        Some(if (name.starts_with("MerchantItem") && name.ends_with("ItemButton")) || name == "MerchantBuyBackItemItemButton" { Kind::Buy } else { Kind::Point })
    }

    /// The backpack slot that the button `ContainerFrame1Item{j}` shows (the buttons are numbered from the bottom right).
    pub(super) fn bag_index(&self, j: usize) -> usize {
        BACKPACK_SIZE - j + self.bag_scroll
    }

    /// What the character carries and what it can carry before slowing down.
    pub fn load(&self) -> (f32, f32) {
        (self.inv.weight(), crate::items::carry_capacity(self.stats.attributes[0]))
    }

    /// The share of the running speed that the load takes (the steed carries it when mounted).
    pub fn burden(&self) -> f32 {
        let (weight, capacity) = self.load();
        if self.mounted { 0.0 } else { crate::items::burden_slowdown(weight, capacity) }
    }

    /// Show an error message at the top of the screen (red, for five seconds).
    /// What the backpack's button `name` holds, if it can be put on the action bar: a usable item.
    pub(super) fn bag_entry_of(&self, name: &str) -> Option<crate::actionbar::Action> {
        let j = name.strip_prefix(&format!("{BACKPACK}Item"))?.parse::<usize>().ok().filter(|j| (1..=BACKPACK_SIZE).contains(j))?;
        let stack = self.inv.bag[self.bag_index(j)]?;
        crate::consume::Consumed::can_use(stack.item).then_some(crate::actionbar::Action::Item(stack.item))
    }

    /// How many of `item` the backpack holds and the seconds before it can be used again, for its action button.
    pub fn item_state(&self, item: usize) -> (u32, f32) {
        (self.inv.count(item), self.consumed.cooldown(item))
    }

    /// If the mouse carries a usable item, put it back in the backpack and give the action it makes for a button.
    pub fn take_carried_action(&mut self) -> Option<crate::actionbar::Action> {
        let item = self.inv.cursor.map(|c| c.item).filter(|&i| crate::consume::Consumed::can_use(i))?;
        self.inv.put_cursor_away();
        self.refresh();
        self.inv.cursor.is_none().then_some(crate::actionbar::Action::Item(item))
    }

    /// An action button asked to use `item`: use the first stack of it in the backpack.
    /// The lure or stone whose cast is to begin now, if one was used.
    pub fn take_lure(&mut self) -> Option<usize> {
        self.lure.0.take()
    }

    /// The cast of a lure is over: it goes on the weapon.
    pub fn finish_lure(&mut self, item: usize) {
        self.lure.1 = true;
        self.use_item_of(item);
        self.lure.1 = false;
    }

    pub fn use_item_of(&mut self, item: usize) {
        if let Some(slot) = self.inv.bag.iter().position(|s| s.is_some_and(|s| s.item == item)) {
            self.use_item(slot);
        }
    }

    /// Right-click on a thing that does something when used: a potion, food, a scroll, the hearthstone.
    pub(super) fn use_item(&mut self, slot: usize) {
        use crate::consume::UseError;
        let Some(stack) = self.inv.bag[slot] else { return };
        let item = crate::items::get(stack.item);
        let name = item.name;
        // A recipe, pattern, plan or formula: using it teaches what it makes, if the trade is far enough along (and it is not used up when it is no use).
        let taught: Vec<u32> = item.spells.iter().flat_map(|s| s.effects.iter()).filter_map(|e| if let crate::items::UseEffect::Teach(spell) = e { Some(*spell) } else { None }).filter(|&s| crate::recipes::get(s).is_some()).collect();
        if !taught.is_empty() {
            return self.learn_recipe_item(stack.item, &taught);
        }
        // A mount asks for a rank of riding (the skill of the original's tables, 762).
        let steed = item.spells.iter().any(|s| s.effects.iter().any(|e| matches!(e, crate::items::UseEffect::Mount(..))));
        // Getting on takes a cast, and the character must stand still and out of a fight for it; getting down is at once.
        if steed && !self.mounted && !self.calm {
            return self.error(&self.string("ERR_NOT_IN_COMBAT"));
        }
        if steed && item.skill.0 == 762 && self.skills.rank(crate::skills::Skill::Riding) < item.skill.1 {
            return self.error(&self.string("ITEM_REQ_SKILL").replacen("%s", &format!("{} ({})", skill_name(762), item.skill.1), 1));
        }
        // The item of a quest: a binding calls the demon to fight, a taming rod tames the beast.
        match self.quests.used(stack.item) {
            Some(crate::quests::QuestUse::Summon(creature)) => return self.requests.push(Request::Foe(creature)),
            Some(crate::quests::QuestUse::Tame(creature)) => return self.requests.push(Request::Tame(creature)),
            None => {}
        }
        // A soulstone keeps the soul of the character: if he dies, he gets up where he fell.
        if item.spells.iter().any(|s| content::classic_spell(s.spell).is_some_and(|c| c.name == "Soulstone Resurrection")) {
            let share = ["Minor", "Lesser", "Greater", "Major"].iter().position(|w| item.name.starts_with(w)).map_or(0.4, |i| [0.2, 0.3, 0.5, 0.6][i]);
            self.inv.remove(stack.item, 1);
            self.consumed.add_timed(crate::consume::Timed::Ward(share), 1800.0);
            self.consumed.show("Spell_Shadow_SoulGem", 1800.0);
            return self.notice("Your soul is kept.");
        }
        // A lure goes on a fishing pole in the hand and asks for the skill it names; a stone or an oil, on a weapon.
        let temp = item.spells.iter().flat_map(|s| s.effects.iter()).find_map(|e| if let crate::items::UseEffect::Temp(enchant, _) = e { Some(*enchant) } else { None });
        if let Some(enchant) = temp {
            let lure = content::get().enchants.get(&enchant).is_some_and(|e| e.effects.iter().any(|e| matches!(e, crate::items::UseEffect::Equip(30, 356, _))));
            if lure && !crate::fishing::pole_in_hand(&self.inv) {
                return self.error(&self.string("SPELL_FAILED_TOTEMS").replacen("%s", "Fishing Pole", 1));
            }
            // The kind of weapon the spell asks for (a sharpening stone: the sharp ones; a weightstone: the blunt ones; a lure: the pole).
            let asked = item.spells.first().and_then(|s| content::classic_spell(s.spell)).and_then(|s| s.equipped);
            let fits = self.inv.equipment[crate::items::MAIN_HAND].is_some_and(|w| {
                let weapon = crate::items::get(w.item);
                asked.is_none_or(|(class, mask)| weapon.class as i32 == class && (mask == 0 || mask & (1 << weapon.subclass) != 0))
            });
            if !fits {
                return self.error(&self.string("SPELL_FAILED_EQUIPPED_ITEM"));
            }
            if let Some(skill) = crate::skills::Skill::of_line(item.skill.0) {
                if !self.skills.knows(skill) || self.skills.rank(skill) < item.skill.1 {
                    return self.error(&self.string("ITEM_REQ_SKILL").replacen("%s", &format!("{} ({})", skill.name(), item.skill.1), 1));
                }
            }
        }
        // It is put on in a cast of its own (the game starts it, and uses the item again when it is over).
        if temp.is_some() && !std::mem::take(&mut self.lure.1) {
            self.lure.0 = Some(stack.item);
            return;
        }
        match self.consumed.use_item(&mut self.inv, slot, self.progress.level, self.calm) {
            Ok(mut used) => {
                self.restored = (self.restored.0 + used.heal, self.restored.1 + used.mana);
                if used.hearth {
                    self.cast_start(name, crate::consume::HEARTH_SECONDS);
                }
                if let Some((enchant, seconds)) = used.temp {
                    let effects = content::get().enchants.get(&enchant).map(|e| e.effects.clone()).unwrap_or_default();
                    let weapon = self.inv.equipment[crate::items::MAIN_HAND].map(|s| s.item);
                    used.stats_changed |= self.consumed.apply_temp(weapon, &effects, seconds);
                    // A poison does nothing to the stats: it is what the blows cast.
                    if content::get().enchants.get(&enchant).is_some_and(|e| !e.procs.is_empty()) {
                        self.consumed.timed.retain(|(t, _)| !matches!(t, crate::consume::Timed::Coat(_)));
                        self.consumed.add_timed(crate::consume::Timed::Coat(enchant), seconds);
                    }
                }
                if let Some(mount) = used.mount {
                    if self.mounted {
                        self.steed = Some(mount);
                    } else {
                        self.mount_cast = Some((MOUNT_SECONDS, mount));
                        self.cast_start(name, MOUNT_SECONDS);
                    }
                }
                self.events.push(Sfx::PutDown(crate::items::get(stack.item).handling));
                if used.stats_changed {
                    self.equipment_changed = true;
                }
                self.refresh();
            }
            Err(UseError::Level(level)) => self.error(&self.string("ERR_CANT_EQUIP_LEVEL_I").replacen("%d", &level.to_string(), 1)),
            Err(UseError::Cooldown) => self.error(&self.string("ERR_ITEM_COOLDOWN")),
            Err(UseError::NotCalm) => self.error(&self.string("ERR_NOT_IN_COMBAT")),
            Err(UseError::NothingToDo) => {}
        }
    }

    /// Use a recipe item: the trade has to be at the rank it asks for, and there has to be something new in it; then the recipes are known and it is used up.
    pub(super) fn learn_recipe_item(&mut self, item: usize, spells: &[u32]) {
        let def = crate::items::get(item);
        if def.level > self.progress.level {
            return self.error(&self.string("ERR_CANT_EQUIP_LEVEL_I").replacen("%d", &def.level.to_string(), 1));
        }
        if let Some(skill) = crate::skills::Skill::of_line(def.skill.0) {
            if !self.skills.knows(skill) || self.skills.rank(skill) < def.skill.1 {
                return self.error(&self.string("ITEM_REQ_SKILL").replacen("%s", &format!("{} ({})", skill.name(), def.skill.1), 1));
            }
        }
        let learned: Vec<&'static content::Recipe> = spells.iter().filter(|&&s| self.recipes.learn(s)).filter_map(|&s| crate::recipes::get(s).map(|r| r.1)).collect();
        if learned.is_empty() {
            return self.error(&self.string("SPELL_FAILED_SPELL_LEARNED"));
        }
        for r in learned {
            let text = self.string("ERR_LEARN_RECIPE_S").replacen("%s", &r.name, 1);
            self.notice(&text);
        }
        self.inv.remove(item, 1);
        self.events.push(Sfx::PutDown(def.handling));
        self.refresh();
    }

    /// Whether the character keeps still and out of a fight, for what asks it (food, drink, bandages, the way home).
    pub fn set_calm(&mut self, calm: bool) {
        self.calm = calm;
    }

    pub fn set_mounted(&mut self, mounted: bool) {
        self.mounted = mounted;
    }

    /// Whether a mount is being cast.
    pub fn mounting(&self) -> bool {
        self.mount_cast.is_some()
    }

    /// Stop the cast of a mount, saying why on the cast bar.
    pub fn cancel_mount(&mut self, why: &str) {
        if self.mount_cast.take().is_some() {
            self.cast_failed(why);
        }
    }

    /// `dt` seconds of what using items began: regeneration, boosts running out, cooldowns, the way home. Returns when the way home ended.
    pub fn tick_items(&mut self, dt: f32) -> Option<crate::consume::WayHome> {
        let tick = self.consumed.tick(dt, self.calm);
        // The mount being cast: moving or a fight breaks it, and at the end the steed is asked for.
        if let Some((left, mount)) = self.mount_cast {
            if !self.calm {
                self.cancel_mount("Interrupted");
            } else if left <= dt {
                self.mount_cast = None;
                self.cast_stop();
                self.steed = Some(mount);
            } else {
                self.mount_cast = Some((left - dt, mount));
            }
        }
        self.restored = (self.restored.0 + tick.heal, self.restored.1 + tick.mana);
        if tick.stats_changed {
            self.equipment_changed = true;
            self.refresh();
        } else if tick.timed_ended {
            self.stats_dirty = true;
            self.refresh();
        }
        match tick.way_home {
            Some(crate::consume::WayHome::Arrived) => self.cast_stop(),
            Some(crate::consume::WayHome::Interrupted) => self.cast_failed("Interrupted"),
            None => {}
        }
        tick.way_home
    }

    /// The steed (display, speed bonus in percent) that a mount item asked for since the last call.
    pub fn take_steed(&mut self) -> Option<(u32, i32)> {
        self.steed.take()
    }

    /// What spells asked of the game since the last call.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// A spell coats the weapon in the hand with an enchantment (a row of `SpellItemEnchantment.dbc`) for some seconds: what it adds to the stats, and what it casts on a blow.
    pub fn coat(&mut self, enchant: u32, seconds: f32) {
        let Some(e) = content::get().enchants.get(&enchant) else { return };
        let weapon = self.inv.equipment[crate::items::MAIN_HAND].map(|s| s.item);
        self.consumed.timed.retain(|(t, _)| !matches!(t, crate::consume::Timed::Coat(_)));
        self.consumed.apply_temp(weapon, &e.effects, seconds);
        self.consumed.add_timed(crate::consume::Timed::Coat(enchant), seconds);
        self.stats_dirty = true;
        self.refresh();
    }

    /// A spell puts a mount under the character (or takes him down from it): the game does it at the next frame.
    pub fn ride(&mut self, display: u32, percent: i32) {
        self.steed = Some((display, percent));
    }

    /// The health and mana that using items gave since the last call.
    pub fn take_restores(&mut self) -> (f32, f32) {
        std::mem::take(&mut self.restored)
    }

    /// Say why an item was not put on (the original's error messages).
    pub(super) fn equip_error(&mut self, e: crate::items::EquipError) {
        use crate::items::EquipError;
        let text = match e {
            EquipError::Level(level) => self.string("ERR_CANT_EQUIP_LEVEL_I").replacen("%d", &level.to_string(), 1),
            EquipError::TwoHanded => self.string("ERR_2HANDED_EQUIPPED"),
            EquipError::PackFull => self.string("ERR_INV_FULL"),
            EquipError::DoesNotGoThere => self.string("ERR_CANT_EQUIP_EVER"),
        };
        self.error(&text);
    }

    pub fn error(&mut self, text: &str) {
        self.message(text, RED);
    }

    /// Show a message that is not an error (yellow): quest progress, rewards.
    pub fn notice(&mut self, text: &str) {
        self.message(text, YELLOW);
    }

    pub(super) fn message(&mut self, text: &str, color: [f32; 4]) {
        self.messages.push((text.to_string(), color, 0.0));
        if self.messages.len() > MAX_MESSAGES {
            self.messages.remove(0);
        }
    }
}
