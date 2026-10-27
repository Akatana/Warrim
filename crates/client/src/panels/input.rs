//! The mouse: hovering, pressing and releasing over the windows.

use super::*;

impl Panels {
    pub fn hover(&mut self, pos: Option<(f32, f32)>, screen: (u32, u32)) {
        self.cursor = pos;
        if let (true, Some(p)) = (self.map_open, pos) {
            self.map_drag(p, screen);
        }
        if let Some(p) = pos {
            self.sleep_drag(p, screen);
            self.slider_drag(p, screen);
        }
        self.tree.hover = pos.and_then(|p| self.tree.hit_test(p, screen));
    }

    /// Whether the position is over a frame of the panels (the game should leave the click to them).
    pub fn is_over(&mut self, pos: (f32, f32), screen: (u32, u32)) -> bool {
        self.map_open || self.level_open || (self.any_open() && self.tree.hit_test(pos, screen).is_some())
    }

    /// A mouse button went down; only the left one lifts a spell or an item off its button to drag it.
    pub fn mouse_down(&mut self, pos: (f32, f32), screen: (u32, u32), left: bool) -> bool {
        if self.level_open {
            self.level_press(pos, screen);
            return true;
        }
        if self.map_open {
            self.map_press(pos, screen);
            return true;
        }
        let hit = self.tree.hit_test(pos, screen);
        self.tree.pressed = hit;
        if hit.is_some_and(|h| self.tree.name(h) == sleep::SLIDER) {
            self.sleep_dragging = true;
            self.sleep_drag(pos, screen);
        }
        if let Some(name) = hit.map(|h| self.tree.name(h).to_string()).filter(|n| n.contains("OptionsFrameSlider")) {
            self.slider_press(&name, pos, screen);
        }
        self.drag = hit.filter(|_| left).and_then(|h| self.book_entry_of(self.tree.name(h)).or_else(|| self.bag_entry_of(self.tree.name(h))));
        hit.is_some()
    }

    /// A mouse button went up over the same button it went down on: do what the click means. `left` or right.
    pub fn mouse_up(&mut self, pos: (f32, f32), screen: (u32, u32), left: bool) {
        if self.level_open {
            if left {
                self.level_release(pos, screen);
            }
            return;
        }
        if self.map_open {
            if left {
                self.map_release(pos, screen);
            } else {
                self.map_zoom_out();
            }
            return;
        }
        self.slider_release();
        self.sleep_dragging = false;
        let down = std::mem::take(&mut self.tree.pressed);
        let Some(hit) = self.tree.hit_test(pos, screen).filter(|h| Some(*h) == down) else { return };
        let name = self.tree.name(hit).to_string();
        match name.as_str() {
            n if self.magic_click(n) || self.menu_click(n) || self.talent_click(n, left) || self.trade_click(n) || self.trainer_click(n) => {}
            "CharacterFrameCloseButton" => {
                self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
                self.events.push(Sfx::Named("igCharacterInfoClose"));
                self.after_close();
            }
            "ContainerFrame1CloseButton" => {
                self.tree.show(BACKPACK, false);
                self.events.push(Sfx::Named("igBackPackClose"));
                self.after_close();
            }
            "LootCloseButton" => self.close_loot(),
            "ItemTextCloseButton" => self.close_book(),
            "ItemTextPrevPageButton" | "ItemTextNextPageButton" => self.turn_book(name == "ItemTextNextPageButton"),
            "GossipFrameCloseButton" | "GossipFrameGreetingGoodbyeButton" => self.close_gossip(),
            "MerchantFrameCloseButton" => self.close_merchant(),
            "QuestLogFrameCloseButton" | "QuestFrameExitButton" => self.toggle_quest_log(),
            "QuestLogFrameAbandonButton" => self.ask_abandon(),
            arrow if arrow.ends_with("ScrollBarScrollUpButton") || arrow.ends_with("ScrollBarScrollDownButton") => {
                let up = arrow.ends_with("UpButton");
                let frame = arrow.trim_end_matches("ScrollUpButton").trim_end_matches("ScrollDownButton").trim_end_matches("ScrollBar").to_string();
                let screen = self.tree.screen();
                self.tree.scroll_by(&frame, screen, if up { 1.0 } else { -1.0 });
            }
            "StaticPopup1Button1" if self.sleep.is_some() => self.sleep_chosen(),
            "StaticPopup1Button1" if self.destroying => self.destroy_asked(),
            "StaticPopup1Button1" => self.abandon_asked(),
            "StaticPopup1Button2" => self.close_popup(),
            button if button.starts_with("QuestFrame") && self.quest_click(button) => {}
            "MerchantBuyBackItemItemButton" => self.buy_back(),
            "MerchantPrevPageButton" | "MerchantNextPageButton" => {
                self.merchant_page = if name == "MerchantNextPageButton" { self.merchant_page + 1 } else { self.merchant_page.saturating_sub(1) };
                self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
                self.fill_merchant();
            }
            "CharacterModelFrameRotateLeftButton" => self.doll_turn -= DOLL_STEP,
            "CharacterModelFrameRotateRightButton" => self.doll_turn += DOLL_STEP,
            _ => {
                let carried = self.inv.cursor;
                if let Some(slot) = SLOT_BUTTONS.iter().position(|b| *b == name) {
                    if left {
                        if let Err(e) = self.inv.click_equipment(slot, self.progress.level) {
                            self.equip_error(e);
                        }
                    } else if self.inv.unequip(slot) {
                        self.events.push(Sfx::PutDown(crate::items::get(carried.map_or(0, |c| c.item)).handling));
                    }
                    self.equipment_changed = true;
                } else if let Some(j) = name.strip_prefix(&format!("{BACKPACK}Item")).and_then(|n| n.parse::<usize>().ok()) {
                    let slot = self.bag_index(j);
                    if left {
                        self.inv.click_bag(slot);
                    } else if self.merchant.is_some() {
                        self.sell_item(slot);
                    } else if let Some(quest) = self.inv.bag[slot].and_then(|s| self.quests.begun_by(s.item, self.who())) {
                        // A letter or a note that begins a quest: the quest is offered as by the one who takes it back.
                        self.open_quest(usize::MAX, &content::get().creatures[content::quest(quest).turn_in], quest, QuestScreen::Detail);
                    } else if self.inv.bag[slot].is_some_and(|s| crate::items::get(s.item).tome.is_some()) {
                        self.read_tome(slot);
                    } else if let Some(read) = self.inv.bag[slot].map(|s| crate::items::get(s.item)).filter(|i| i.page != 0) {
                        self.open_book(read.name, read.page, read.material);
                    } else if self.inv.bag[slot].is_some_and(|s| crate::consume::Consumed::can_use(s.item)) {
                        self.use_item(slot);
                    } else if let Some(worn) = self.inv.bag[slot] {
                        match self.inv.equip_from_bag(slot, self.progress.level) {
                            Ok(()) => {
                                self.equipment_changed = true;
                                self.events.push(Sfx::PutDown(crate::items::get(worn.item).handling));
                            }
                            // What does not go on the body (a potion, a bag of cloth) is simply not worn.
                            Err(crate::items::EquipError::DoesNotGoThere) => {}
                            Err(e) => self.equip_error(e),
                        }
                    }
                } else if let Some(i) = name.strip_prefix("LootButton").and_then(|n| n.parse::<usize>().ok()) {
                    self.take_loot(i);
                } else if let Some(i) = name.strip_prefix("MerchantItem").and_then(|n| n.strip_suffix("ItemButton")).and_then(|n| n.parse::<usize>().ok()) {
                    self.buy_ware(i);
                } else if let Some(i) = name.strip_prefix("QuestRewardItem").and_then(|n| n.parse::<usize>().ok()) {
                    // On the reward screen an item to choose from is picked by clicking it (the highlight moves to it).
                    let choices = self.questing.map_or(0, |(_, _, quest, _)| content::quest(quest).choice.len());
                    if i >= 1 && i <= choices {
                        self.reward_choice = Some(i - 1);
                        self.tree.set_point("QuestRewardItemHighlight", Point::TopLeft, Some(&name), Some(Point::TopLeft), -8.0, 7.0);
                        self.tree.show("QuestRewardItemHighlight", true);
                    }
                } else if let Some(i) = name.strip_prefix("QuestLogTitle").and_then(|n| n.parse::<usize>().ok()) {
                    self.select_log(i);
                } else if let Some(i) = name.strip_prefix("GossipTitleButton").and_then(|n| n.parse::<usize>().ok()) {
                    self.choose_gossip(i);
                }
                // What the mouse carries changed: the item it picked up or put down makes its sound.
                match (carried, self.inv.cursor) {
                    (None, Some(now)) => self.events.push(Sfx::PickUp(crate::items::get(now.item).handling)),
                    (Some(before), _) if self.inv.cursor != carried => self.events.push(Sfx::PutDown(crate::items::get(before.item).handling)),
                    _ => {}
                }
                self.refresh();
            }
        }
    }
}
