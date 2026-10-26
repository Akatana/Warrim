//! The quest window: the offer, the progress and the reward.

use super::*;

impl Panels {
    // ---- the quest window

    /// Open the quest window on `quest` of creature `creature` (kind `def`), at one of its three screens.
    pub fn open_quest(&mut self, creature: usize, def: &'static CreatureDef, quest: usize, screen: QuestScreen) {
        self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
        self.hide_log();
        self.hide_gossip();
        self.questing = Some((creature, def, quest, screen));
        self.tree.show(QUEST, true);
        self.events.push(Sfx::Named("igQuestListSelect"));
        self.fill_quest();
    }

    pub(super) fn hide_quest(&mut self) {
        self.questing = None;
        self.tree.show(QUEST, false);
    }

    pub fn close_quest(&mut self) {
        if self.questing.is_some() {
            self.hide_quest();
            self.events.push(Sfx::Named("igQuestListClose"));
        }
        self.after_close();
    }

    /// Enable or disable a button, greying its text.
    pub(super) fn set_button(&mut self, button: &str, enabled: bool) {
        self.tree.set_enabled(button, enabled);
        self.tree.set_text_color(&format!("{button}Text"), if enabled { GOLD } else { GREY });
    }

    /// The reward part of the detail, reward and log screens (`QuestFrameItems_Update`): the title, the items to choose from
    /// ("Choose your reward:"), the line "You will receive:" with the money next to it, and the items, two to a row. The
    /// buttons number the items to choose from first, then those everyone gets.
    pub(super) fn fill_rewards(&mut self, prefix: &str, quest: usize) {
        let q = content::quest(quest);
        let (choices, gifts) = (q.choice.len(), q.items.len());
        let any = choices + gifts > 0 || q.copper > 0;
        let (title, choose, receive) = (format!("{prefix}RewardTitleText"), format!("{prefix}ItemChooseText"), format!("{prefix}ItemReceiveText"));
        self.tree.set_region_hidden(&title, !any);
        self.tree.set_region_hidden(&choose, choices == 0);
        self.tree.set_text_color(&choose, PARCHMENT);
        // The line before the gifts hangs off the last row of choices, or the title when there are none.
        let (receive_text, above) = if choices > 0 {
            let last = if choices % 2 == 0 { choices - 1 } else { choices };
            (self.string("REWARD_ITEMS"), (format!("{prefix}Item{last}"), 3.0))
        } else {
            (self.string("REWARD_ITEMS_ONLY"), (title.clone(), 3.0))
        };
        self.tree.set_region_hidden(&receive, gifts == 0 && q.copper == 0);
        self.tree.set_text(&receive, &receive_text);
        self.tree.set_region_point(&receive, Point::TopLeft, Some(&above.0), Some(Point::BottomLeft), above.1, -5.0);
        self.tree.show(&format!("{prefix}MoneyFrame"), q.copper > 0);
        if q.copper > 0 {
            self.tree.set_money(&format!("{prefix}MoneyFrame"), q.copper, false, false);
        }
        for i in 1..=QUEST_ITEMS {
            let button = format!("{prefix}Item{i}");
            let Some(&stack) = q.rewards().nth(i - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            self.show_stack(&button, stack);
            // Two to a row, the first row under the line that introduces its kind of items.
            let (local, first_of_kind) = if i <= choices { (i, 1) } else { (i - choices, choices + 1) };
            let heading = if i <= choices { choose.clone() } else { receive.clone() };
            if i == first_of_kind {
                self.tree.set_point(&button, Point::TopLeft, Some(&heading), Some(Point::BottomLeft), -3.0, -5.0);
            } else if local % 2 == 1 {
                self.tree.set_point(&button, Point::TopLeft, Some(&format!("{prefix}Item{}", i - 2)), Some(Point::BottomLeft), 0.0, -2.0);
            } else {
                self.tree.set_point(&button, Point::TopLeft, Some(&format!("{prefix}Item{}", i - 1)), Some(Point::TopRight), 1.0, 0.0);
            }
        }
    }

    /// Put a stack on a quest item button: its picture, name and (above one) count.
    pub(super) fn show_stack(&mut self, button: &str, stack: Stack) {
        let item = crate::items::get(stack.item);
        self.tree.set_texture(&format!("{button}IconTexture"), Some(item.icon));
        self.tree.set_text(&format!("{button}Name"), item.name);
        self.tree.set_text(&format!("{button}Count"), &stack.count.to_string());
        self.tree.set_region_hidden(&format!("{button}Count"), stack.count <= 1);
        self.tree.show(button, true);
    }

    /// `QuestFrameDetailPanel_OnShow` and its two sisters: the texts of the quest and the buttons of the screen.
    pub(super) fn fill_quest(&mut self) {
        let Some((_, def, quest, screen)) = self.questing else { return };
        for scroll in ["QuestDetailScrollFrame", "QuestProgressScrollFrame", "QuestRewardScrollFrame"] {
            self.tree.reset_scroll(scroll);
        }
        let q = content::quest(quest);
        let (name, race) = (self.name.clone(), self.race);
        let say = |text: &str| speak_to(&name, race, text);
        for (panel, which) in [("QuestFrameDetailPanel", QuestScreen::Detail), ("QuestFrameProgressPanel", QuestScreen::Progress), ("QuestFrameRewardPanel", QuestScreen::Reward)] {
            self.tree.show(panel, which == screen);
        }
        self.tree.set_text("QuestFrameNpcNameText", &def.name);
        let texts: Vec<(&str, String)> = match screen {
            QuestScreen::Detail => vec![("QuestTitleText", q.title.clone()), ("QuestDescription", say(&q.details)), ("QuestObjectiveText", say(&q.objective))],
            QuestScreen::Progress => vec![("QuestProgressTitleText", q.title.clone()), ("QuestProgressText", say(&q.progress))],
            QuestScreen::Reward => vec![("QuestRewardTitleText", q.title.clone()), ("QuestRewardText", say(&q.completion))],
        };
        for (name, text) in &texts {
            self.tree.set_text(name, text);
            self.tree.set_text_color(name, PARCHMENT);
        }
        match screen {
            QuestScreen::Detail => {
                for name in ["QuestDetailObjectiveTitleText", "QuestDetailRewardTitleText", "QuestDetailItemReceiveText"] {
                    self.tree.set_text_color(name, PARCHMENT);
                }
                self.fill_rewards("QuestDetail", quest);
            }
            QuestScreen::Reward => {
                self.reward_choice = None;
                self.tree.show("QuestRewardItemHighlight", false);
                for name in ["QuestRewardRewardTitleText", "QuestRewardItemReceiveText"] {
                    self.tree.set_text_color(name, PARCHMENT);
                }
                self.fill_rewards("QuestReward", quest);
            }
            QuestScreen::Progress => {
                // The items still to bring (the kill goals show in the log and the tracker).
                let wanted: Vec<Stack> = q.goals.iter().filter_map(|g| if let Goal::Collect { item, count } = *g { Some(Stack { item, count, enchant: 0 }) } else { None }).collect();
                self.tree.set_text_color("QuestProgressRequiredItemsText", PARCHMENT);
                self.tree.set_region_hidden("QuestProgressRequiredItemsText", wanted.is_empty());
                self.tree.set_region_hidden("QuestProgressRequiredMoneyText", true);
                self.tree.show("QuestProgressRequiredMoneyFrame", false);
                self.tree.set_point("QuestProgressItem1", Point::TopLeft, Some("QuestProgressRequiredItemsText"), Some(Point::BottomLeft), -3.0, -5.0);
                for i in 1..=PROGRESS_ITEMS {
                    match wanted.get(i - 1) {
                        Some(&stack) => self.show_stack(&format!("QuestProgressItem{i}"), stack),
                        None => self.tree.show(&format!("QuestProgressItem{i}"), false),
                    }
                }
                let ready = self.quests.complete(quest, &self.inv);
                self.set_button("QuestFrameCompleteButton", ready);
            }
        }
    }

    /// The buttons of the quest window.
    pub(super) fn quest_click(&mut self, button: &str) -> bool {
        let Some((creature, def, quest, _)) = self.questing else { return false };
        let q = content::quest(quest);
        match button {
            "QuestFrameAcceptButton" => {
                if self.quests.accept(quest) {
                    for stack in &q.start_items {
                        self.inv.add(*stack);
                    }
                    self.notice(&self.string("ERR_QUEST_ACCEPTED_S").replacen("%s", &q.title, 1));
                    self.events.push(Sfx::Named("QUESTADDED"));
                    self.refresh_quest_marks(quest);
                    self.hide_quest();
                    self.after_close();
                } else {
                    self.error(&self.string("ERR_QUEST_LOG_FULL"));
                }
            }
            "QuestFrameDeclineButton" | "QuestFrameGoodbyeButton" | "QuestFrameCancelButton" => {
                self.hide_quest();
                self.events.push(Sfx::Named("igQuestCancel"));
                self.after_close();
            }
            "QuestFrameCloseButton" => self.close_quest(),
            "QuestFrameCompleteButton" => {
                self.questing = Some((creature, def, quest, QuestScreen::Reward));
                self.fill_quest();
            }
            "QuestFrameCompleteQuestButton" if !q.choice.is_empty() && self.reward_choice.is_none() => self.error(&self.string("ERR_QUEST_MUST_CHOOSE")),
            "QuestFrameCompleteQuestButton" => match self.quests.turn_in(quest, &mut self.inv, self.reward_choice) {
                Ok(()) => {
                    self.events.push(Sfx::Named("igQuestListComplete"));
                    self.ready.remove(&quest);
                    if q.copper > 0 {
                        self.notice(&self.string("ERR_QUEST_REWARD_MONEY_S").replacen("%s", &coins(q.copper), 1));
                    }
                    let xp = crate::levels::quest_xp(q.xp, q.level, self.progress.level);
                    if xp > 0 {
                        self.notice(&self.string("ERR_QUEST_REWARD_EXP_I").replacen("%d", &xp.to_string(), 1));
                        self.gain_xp(xp);
                    }
                    for stack in self.reward_choice.map(|i| q.choice[i]).iter().chain(&q.items) {
                        self.notice(&self.string("ERR_QUEST_REWARD_ITEM_S").replacen("%s", crate::items::get(stack.item).name, 1));
                    }
                    // The creature may have the next quest of the chain for the player right away.
                    match self.quests.offered_by(kind_of(def), self.who()).first() {
                        Some(&next) => self.open_quest(creature, def, next, QuestScreen::Detail),
                        None => {
                            self.hide_quest();
                            self.after_close();
                        }
                    }
                }
                Err(why) => self.refuse(why),
            },
            _ => return false,
        }
        true
    }

    /// A taken quest starts counting from what the player carries already, and is announced complete only once.
    pub(super) fn refresh_quest_marks(&mut self, quest: usize) {
        for (g, goal) in content::quest(quest).goals.iter().enumerate() {
            if matches!(goal, Goal::Collect { .. }) {
                self.seen.insert((quest, g), self.quests.have(quest, g, &self.inv));
            }
        }
        if self.quests.complete(quest, &self.inv) {
            self.ready.insert(quest);
        }
    }
}
