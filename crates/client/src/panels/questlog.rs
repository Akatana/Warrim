//! The quest log window and the abandon-quest popup.

use super::*;

impl Panels {
    /// Ask whether the selected quest of the log is to be abandoned (`StaticPopupDialogs["ABANDON_QUEST"]`).
    pub(super) fn ask_abandon(&mut self) {
        let Some(active) = self.quests.active.get(self.log_selected) else { return };
        self.popup = Some(active.quest);
        let question = self.string("ABANDON_QUEST_CONFIRM").replacen("%s", &content::quest(active.quest).title, 1);
        self.tree.set_text("StaticPopup1Text", &question);
        self.tree.set_text("StaticPopup1Button1Text", &self.string("YES"));
        self.tree.set_text("StaticPopup1Button2Text", &self.string("NO"));
        self.tree.show(sleep::SLIDER, false);
        self.tree.show(POPUP, true);
    }

    pub fn close_popup(&mut self) {
        self.popup = None;
        self.sleep = None;
        // "No" (or Esc) to destroying: the item goes back in the pack.
        if std::mem::take(&mut self.destroying) {
            self.inv.put_cursor_away();
            self.refresh();
        }
        self.tree.show(POPUP, false);
        self.after_close();
    }

    /// `Esc`: the question goes first, then everything else.
    pub fn escape(&mut self) {
        if self.level_open {
            self.toggle_level();
        } else if self.popup_open() {
            self.close_popup();
        } else if !self.escape_menu() {
            self.close_all();
        }
    }

    // ---- the quest log

    /// `ToggleQuestLog`. Like the character sheet it is a left window: the others close for it.
    pub fn toggle_quest_log(&mut self) {
        let on = !self.quest_log_open();
        if on {
            self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
            self.tree.show(trade::TRADE, false);
            self.close_loot();
            self.close_book();
            self.hide_gossip();
            self.close_merchant();
            self.hide_quest();
        }
        self.tree.show(QUESTLOG, on);
        self.tree.reset_scroll("QuestLogDetailScrollFrame");
        if !on {
            self.tree.show(POPUP, false);
            self.popup = None;
        }
        self.events.push(Sfx::Named(if on { "igQuestLogOpen" } else { "igQuestLogClose" }));
        self.fill_log();
        self.after_close();
    }

    pub(super) fn hide_log(&mut self) {
        self.tree.show(QUESTLOG, false);
        self.tree.show(POPUP, false);
        self.popup = None;
        self.sleep = None;
    }

    /// `QuestLog_Update` and `QuestLog_UpdateQuestDetails`: the list of quests, and the selected one in detail.
    pub(super) fn fill_log(&mut self) {
        if !self.quest_log_open() {
            return;
        }
        let count = self.quests.active.len();
        self.log_selected = self.log_selected.min(count.saturating_sub(1));
        self.tree.show("EmptyQuestLogFrame", count == 0);
        self.tree.show("QuestLogDetailScrollFrame", count > 0);
        self.set_button("QuestLogFrameAbandonButton", count > 0);
        let total = self.string("QUEST_LOG_COUNT_TEMPLATE").replacen("%d", &count.to_string(), 1).replacen("%d", &crate::quests::MAX_ACTIVE.to_string(), 1);
        self.tree.set_text("QuestLogQuestCount", &total);
        for i in 1..=LOG_LINES {
            let button = format!("QuestLogTitle{i}");
            let Some(active) = self.quests.active.get(i - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            let quest = active.quest;
            let mark = if self.quests.tracked == Some(quest) { "> " } else { "  " };
            self.tree.set_text(&format!("{button}NormalText"), &format!("{mark}{}", content::quest(quest).title));
            self.tree.set_text(&format!("{button}Tag"), if self.quests.complete(quest, &self.inv) { "(Complete)" } else { "" });
            // Only the headers of zones have the plus and minus pictures; a quest is just its line.
            self.tree.hide_button_art(&button);
            self.tree.set_region_hidden(&format!("{button}Highlight"), true);
            // Yellow: a quest of the player's own level (`QuestDifficultyColor`).
            for part in ["NormalText", "Tag"] {
                self.tree.set_text_color(&format!("{button}{part}"), YELLOW);
            }
            self.tree.show(&button, true);
        }
        self.tree.show("QuestLogHighlightFrame", count > 0 && self.log_selected < LOG_LINES);
        if count > 0 {
            self.tree.set_point("QuestLogHighlightFrame", Point::TopLeft, Some(&format!("QuestLogTitle{}", self.log_selected + 1)), Some(Point::TopLeft), 5.0, 0.0);
            self.fill_log_details();
        }
    }

    pub(super) fn fill_log_details(&mut self) {
        let Some(active) = self.quests.active.get(self.log_selected) else { return };
        let (quest, q) = (active.quest, content::quest(active.quest));
        let black = [0.0, 0.0, 0.0, 1.0];
        self.tree.set_text("QuestLogQuestTitle", &q.title);
        self.tree.set_text("QuestLogObjectivesText", &self.speak(&q.objective));
        self.tree.set_text("QuestLogQuestDescription", &self.speak(&q.details));
        for i in 1..=LOG_GOALS {
            let line = format!("QuestLogObjective{i}");
            let Some(goal) = q.goals.get(i - 1) else {
                self.tree.set_region_hidden(&line, true);
                continue;
            };
            let (text, need) = match *goal {
                Goal::Kill { creature, count } => (self.counted_text("ERR_QUEST_ADD_KILL_SII", &content::get().creatures[creature].name, quest, i - 1, count), count),
                Goal::Collect { item, count } => (self.counted_text("ERR_QUEST_ADD_ITEM_SII", crate::items::get(item).name, quest, i - 1, count), count),
                Goal::Reach { .. } => (format!("Reach the destination: {}/1", self.quests.have(quest, i - 1, &self.inv)), 1),
            };
            let finished = self.quests.have(quest, i - 1, &self.inv) >= need;
            self.tree.set_text(&line, &if finished { format!("{text} ({})", self.string("COMPLETE")) } else { text });
            self.tree.set_text_color(&line, if finished { [0.2, 0.2, 0.2, 1.0] } else { black });
            self.tree.set_region_hidden(&line, false);
            if i == 1 {
                self.tree.set_region_point(&line, Point::TopLeft, Some("QuestLogObjectivesText"), Some(Point::BottomLeft), 0.0, -10.0);
            } else {
                self.tree.set_region_point(&line, Point::TopLeft, Some(&format!("QuestLogObjective{}", i - 1)), Some(Point::BottomLeft), 0.0, -2.0);
            }
        }
        let last = if q.goals.is_empty() { "QuestLogObjectivesText".to_string() } else { format!("QuestLogObjective{}", q.goals.len()) };
        self.tree.set_region_point("QuestLogDescriptionTitle", Point::TopLeft, Some(&last), Some(Point::BottomLeft), 0.0, -10.0);
        self.tree.set_region_hidden("QuestLogRequiredMoneyText", true);
        self.tree.show("QuestLogRequiredMoneyFrame", false);
        self.fill_rewards("QuestLog", quest);
    }

    /// The text of goal `goal` of a quest as the messages and the log say it: `"Wolf slain: 1/2"`.
    pub(super) fn counted_text(&self, key: &str, name: &str, quest: usize, goal: usize, need: u32) -> String {
        self.counted(key, name, self.quests.have(quest, goal, &self.inv), need)
    }

    pub(super) fn select_log(&mut self, line: usize) {
        if line <= self.quests.active.len() {
            // Clicking a quest makes it the one the compass points to; clicking the selected one again lets go of it.
            let quest = self.quests.active[line - 1].quest;
            self.quests.tracked = if self.log_selected == line - 1 && self.quests.tracked == Some(quest) { None } else { Some(quest) };
            self.log_selected = line - 1;
            self.tree.reset_scroll("QuestLogDetailScrollFrame");
            self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
            self.fill_log();
        }
    }

    /// The answer "yes" to the question: the quest is given up.
    pub(super) fn abandon_asked(&mut self) {
        let Some(quest) = self.popup else { return };
        self.tree.show(POPUP, false);
        self.popup = None;
        self.quests.abandon(quest);
        for stack in &content::quest(quest).start_items {
            self.inv.remove(stack.item, stack.count);
        }
        self.ready.remove(&quest);
        self.events.push(Sfx::Named("igQuestLogAbandonQuest"));
        self.fill_log();
        self.after_close();
    }
}
