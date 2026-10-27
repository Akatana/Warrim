//! What a creature says when spoken to, and the choices it offers.

use super::*;

impl Panels {
    /// Open the talk window for creature `creature` of kind `def`. The character sheet and the loot window close.
    pub fn open_gossip(&mut self, creature: usize, def: &'static CreatureDef) {
        self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
        self.close_book();
        self.close_loot();
        self.close_merchant();
        self.hide_quest();
        self.hide_log();
        self.gossip = Some((creature, def, None));
        self.tree.show(GOSSIP, true);
        self.events.push(Sfx::Named("igQuestListOpen"));
        self.fill_gossip();
    }

    pub fn close_gossip(&mut self) {
        if self.gossip.is_some() {
            self.events.push(Sfx::Named("igQuestListClose"));
        }
        self.hide_gossip();
        self.after_close();
    }

    pub(super) fn hide_gossip(&mut self) {
        self.gossip = None;
        self.tree.show(GOSSIP, false);
    }

    /// `GossipFrameUpdate`: the name, the text (the greeting, or the last answer) and a button for each line to choose:
    /// the quests the creature gives, the quests it takes back, then what else it offers.
    pub(super) fn fill_gossip(&mut self) {
        let Some((_, def, said)) = &self.gossip else { return };
        let Some(gossip) = &def.gossip else { return };
        let (def, answered) = (*def, said.is_some());
        let text = self.speak(said.as_deref().unwrap_or(&gossip.greeting));
        let kind = kind_of(def);
        let mut lines = Vec::new();
        if !answered {
            lines.extend(self.quests.offered_by(kind, self.who()).into_iter().map(Line::Available));
            lines.extend(self.quests.taken_by(kind, &self.inv).into_iter().map(|(quest, _)| Line::Active(quest)));
            lines.extend((0..gossip.options.len()).map(Line::Option));
        }
        self.tree.set_text("GossipFrameNpcNameText", &def.name);
        self.tree.set_text("GossipGreetingText", &text);
        for i in 1..=GOSSIP_BUTTONS {
            let button = format!("GossipTitleButton{i}");
            if let Some(&line) = lines.get(i - 1) {
                let (title, icon) = match line {
                    Line::Available(q) => (content::quest(q).title.clone(), "AvailableQuestIcon"),
                    Line::Active(q) => (content::quest(q).title.clone(), "ActiveQuestIcon"),
                    Line::Option(o) => (gossip.options[o].text.clone(), match gossip.options[o].action {
                        Action::Say(_) => "GossipGossipIcon",
                        Action::Vendor => "VendorGossipIcon",
                        Action::Train { .. } | Action::Teach => "TrainerGossipIcon",
                    }),
                };
                self.tree.set_text(&format!("{button}Text"), &title);
                self.tree.set_texture(&format!("{button}GossipIcon"), Some(&format!(r"Interface\GossipFrame\{icon}")));
            }
            self.tree.show(&button, i <= lines.len());
        }
        self.gossip_lines = lines;
    }

    /// Choose line `n` (counted from 1) of what the creature offers.
    pub(super) fn choose_gossip(&mut self, n: usize) {
        let Some(&line) = self.gossip_lines.get(n - 1) else { return };
        let Some((creature, def, said)) = &mut self.gossip else { return };
        let (creature, def) = (*creature, *def);
        let mut trained = None;
        match line {
            Line::Available(quest) => return self.open_quest(creature, def, quest, QuestScreen::Detail),
            Line::Active(quest) => {
                // As in the original: a quest that is done and asks for no items goes straight to its reward (the window of what is still missing only comes with items).
                let ready = self.quests.complete(quest, &self.inv) && !content::quest(quest).goals.iter().any(|g| matches!(g, content::Goal::Collect { .. }));
                return self.open_quest(creature, def, quest, if ready { QuestScreen::Reward } else { QuestScreen::Progress });
            }
            Line::Option(o) => match &def.gossip.as_ref().expect("a gossip line has gossip").options[o].action {
                Action::Say(reply) => *said = Some(reply.clone()),
                Action::Vendor => return self.open_merchant(creature, def),
                Action::Teach => return self.open_trainer(creature, def),
                Action::Train { skill, rank, needs, level, cost } => trained = Some((*skill, *rank, *needs, *level, *cost)),
            },
        }
        if let Some((skill, rank, needs, level, cost)) = trained {
            let reply = self.learn_skill(skill, rank, needs, level, cost);
            if let Some((_, _, said)) = &mut self.gossip {
                *said = Some(reply);
            }
        }
        self.fill_gossip();
    }

    /// An instructor teaches `skill` up to `rank` for `cost` copper: the answer he gives. It is refused to a character who is too low a level, has not
    /// learned the step before (`needs`) or has too little money.
    pub(super) fn learn_skill(&mut self, skill: crate::skills::Skill, rank: u32, needs: u32, level: u32, cost: u32) -> String {
        if self.skills.rank(skill) >= rank {
            return "You have learned all of that already.".into();
        }
        if self.progress.level < level {
            self.error(&self.string("ERR_CANT_EQUIP_LEVEL_I").replacen("%d", &level.to_string(), 1));
            return format!("Come back when you have reached level {level}, $N.");
        }
        if self.skills.rank(skill) < needs {
            return "You must learn the step before this one first.".into();
        }
        if self.inv.money < cost {
            self.error(&self.string("ERR_NOT_ENOUGH_MONEY"));
            return "That is my price, $N. Come back when you can pay it.".into();
        }
        self.inv.money -= cost;
        self.skills.learn(skill, rank);
        self.notice(&self.string("ERR_SKILL_GAINED_S").replacen("%s", skill.name(), 1));
        self.events.push(Sfx::Named("igSpellBookSpellIconPickup"));
        self.refresh();
        self.fill_skills();
        "Well done. Sit up straight, keep your heels down, and the horse will do the rest.".into()
    }
}
