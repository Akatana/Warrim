//! The trainer window: the original's `ClassTrainerFrame` (the `Blizzard_TrainerUI` addon) as the trade trainers use it. It lists what the trainer teaches
//! (`classic_trainers.ron`) in the colours of `ClassTrainerFrame_Update`: green for what can be learned, red for what asks for more (level or rank of the trade), and
//! left out what is known (the original's filter says so); the selected lesson shows what it asks for and costs, and Train pays and teaches the recipe.

use super::*;
use crate::recipes;

pub const TRAINER: &str = "ClassTrainerFrame";
const ROWS: usize = 10;

/// What the window keeps: the creature taught by (its place and kind), the lesson selected (a place in the trainer's list) and the first line shown.
#[derive(Default)]
pub struct Trainer {
    who: Option<(usize, &'static CreatureDef)>,
    selected: Option<usize>,
    offset: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Standing {
    Available,
    Unavailable,
}

/// Load the window as the trade trainers use it (ten lines, no headings, no filter).
pub fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate(TRAINER)?;
    tree.show(TRAINER, false);
    for part in ["ClassTrainerFrameFilterDropDown", "ClassTrainerExpandButtonFrame", "ClassTrainerSkill11"] {
        tree.show(part, false);
    }
    tree.set_size("ClassTrainerListScrollFrame", None, Some(168.0));
    tree.set_size("ClassTrainerDetailScrollFrame", None, Some(135.0));
    tree.set_region_point("ClassTrainerHorizontalBarLeft", Point::TopLeft, Some(TRAINER), Some(Point::TopLeft), 15.0, -259.0);
    Ok(())
}

/// What a lesson is called: the recipe, or the step of the trade.
fn lesson_name(l: &content::Lesson) -> String {
    match (l.step, recipes::get(l.spell)) {
        (None, Some((_, r))) => r.name.clone(),
        _ => content::classic_spell(l.spell).map_or_else(|| "?".to_string(), |s| s.name.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;

    #[test]
    fn mining_herbalism_and_skinning_can_be_learned_from_the_first_level() {
        let c = content::get();
        for skill in [Skill::Mining, Skill::Herbalism, Skill::Skinning] {
            let apprentices: Vec<&content::Lesson> = c.lessons.values().flatten().filter(|l| l.step == Some((skill, 1))).collect();
            assert!(!apprentices.is_empty() && apprentices.iter().all(|l| l.level <= 1 && l.skill.is_none()), "{skill:?}");
        }
    }
}

impl Panels {
    pub fn trainer_open(&self) -> bool {
        self.trainer.who.is_some()
    }

    /// For the headless picture: a newcomer of level 12 with some silver, in front of the trainer of Elwynn's farm (`--panel trainer`).
    pub fn trainer_preview(&mut self) {
        self.set_level(12);
        self.inv.money = 5_000;
        let c = content::get();
        let at = content::creature_index("c223");
        self.open_trainer(at, &c.creatures[at]);
    }

    /// The creature being taught by (for the window to close when the character walks away).
    pub fn trainer_creature(&self) -> Option<usize> {
        self.trainer.who.map(|(creature, _)| creature)
    }

    /// The lessons that are shown, as places in the trainer's list: what can be learned, then what asks for more; what is known is left out.
    fn trainer_rows(&self) -> Vec<(usize, Standing)> {
        let Some((_, def)) = self.trainer.who else { return Vec::new() };
        let c = content::get();
        let Some(lessons) = c.lessons.get(&def.id) else { return Vec::new() };
        let mut rows: Vec<(usize, Standing)> = lessons
            .iter()
            .enumerate()
            .filter(|(_, l)| match l.step {
                // A step of a trade is shown until it is learned.
                Some((skill, step)) => self.skills.step(skill) < step,
                None => recipes::get(l.spell).is_some_and(|(_, r)| !self.recipes.knows(r)),
            })
            .map(|(i, l)| {
                // A recipe asks for its trade to be learned; a step asks for the step before it.
                let taught = match (l.step, recipes::get(l.spell)) {
                    (Some((skill, step)), _) => self.skills.step(skill) + 1 == step,
                    (None, Some((_, r))) => self.skills.knows(r.skill),
                    _ => false,
                };
                let ready = taught && self.progress.level >= l.level && l.skill.is_none_or(|(skill, rank)| self.skills.rank(skill) >= rank);
                (i, if ready { Standing::Available } else { Standing::Unavailable })
            })
            .collect();
        rows.sort_by_key(|&(i, standing)| (standing != Standing::Available, lessons[i].skill.map_or(0, |s| s.1), lessons[i].level, i));
        rows
    }

    /// Open the window for creature `creature` of kind `def`.
    pub fn open_trainer(&mut self, creature: usize, def: &'static CreatureDef) {
        self.gossip = None;
        self.tree.show(GOSSIP, false);
        self.trainer = Trainer { who: Some((creature, def)), selected: None, offset: 0 };
        self.tree.show(TRAINER, true);
        self.events.push(Sfx::Named("igCharacterInfoOpen"));
        self.trainer.selected = self.trainer_rows().first().map(|r| r.0);
        self.fill_trainer();
    }

    pub fn close_trainer(&mut self) {
        if self.trainer.who.take().is_some() {
            self.tree.show(TRAINER, false);
            self.events.push(Sfx::Named("igCharacterInfoClose"));
            self.after_close();
        }
    }

    /// `ClassTrainerFrame_Update` and `ClassTrainer_SetSelection`.
    pub(super) fn fill_trainer(&mut self) {
        let Some((_, def)) = self.trainer.who else { return };
        let c = content::get();
        let lessons: &[content::Lesson] = c.lessons.get(&def.id).map_or(&[], |l| l.as_slice());
        let rows = self.trainer_rows();
        self.trainer.offset = self.trainer.offset.min(rows.len().saturating_sub(ROWS));
        let greeting = def.gossip.as_ref().map_or_else(String::new, |g| self.speak(&g.greeting));
        self.tree.set_text("ClassTrainerNameText", &def.name);
        self.tree.set_text("ClassTrainerGreetingText", &greeting);
        self.tree.show("ClassTrainerHighlightFrame", false);
        self.tree.show("ClassTrainerSkillHighlightFrame", false);
        for i in 1..=ROWS {
            let button = format!("ClassTrainerSkill{i}");
            let Some(&(l, standing)) = rows.get(self.trainer.offset + i - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            let name = lesson_name(&lessons[l]);
            let color = if standing == Standing::Available { [0.0, 1.0, 0.0, 1.0] } else { [0.9, 0.0, 0.0, 1.0] };
            self.tree.show(&button, true);
            self.tree.set_button_texture(&button, None);
            self.tree.set_text(&format!("{button}Text"), &format!("  {name}"));
            self.tree.set_text_color(&format!("{button}Text"), color);
            self.tree.show(&format!("{button}SubText"), false);
            if self.trainer.selected == Some(l) {
                self.tree.set_point("ClassTrainerSkillHighlightFrame", Point::TopLeft, Some(&button), Some(Point::TopLeft), 0.0, 0.0);
                self.tree.set_region_tint("ClassTrainerSkillHighlight", color);
                self.tree.show("ClassTrainerSkillHighlightFrame", true);
            }
        }
        let more = rows.len().saturating_sub(ROWS);
        let bar = "ClassTrainerListScrollFrameScrollBar";
        self.tree.show(bar, more > 0);
        let screen = self.tree.screen();
        if let (true, Some(track)) = (more > 0, self.tree.rect_of(bar, screen)) {
            let travel = (track.h - 16.0).max(0.0);
            self.tree.set_region_point(&format!("{bar}ThumbTexture"), Point::TopLeft, Some(bar), Some(Point::TopLeft), 0.0, -travel * self.trainer.offset as f32 / more as f32);
        }
        self.fill_trainer_detail(&rows, lessons);
    }

    /// The lesson selected: its name, picture, what it asks for (what is not met in red), its price and what it makes.
    fn fill_trainer_detail(&mut self, rows: &[(usize, Standing)], lessons: &[content::Lesson]) {
        let selected = self.trainer.selected.filter(|s| rows.iter().any(|r| r.0 == *s));
        let Some(lesson) = selected.map(|s| &lessons[s]) else {
            for part in ["ClassTrainerSkillName", "ClassTrainerSkillIcon", "ClassTrainerSkillRequirements", "ClassTrainerSkillDescription", "ClassTrainerDetailMoneyFrame", "ClassTrainerCostLabel"] {
                self.tree.show(part, false);
            }
            self.tree.set_enabled("ClassTrainerTrainButton", false);
            return;
        };
        let text = |met: bool, s: String| if met { s } else { format!("|cffff2020{s}|r") };
        let mut needs = Vec::new();
        if lesson.level > 1 {
            needs.push(text(self.progress.level >= lesson.level, format!("Level {}", lesson.level)));
        }
        if let Some((skill, rank)) = lesson.skill {
            needs.push(text(self.skills.rank(skill) >= rank, format!("{} ({rank})", skill.name())));
        }
        let recipe = recipes::get(lesson.spell).map(|r| r.1);
        // A recipe shows what it makes; a step of a trade the picture of its spell.
        let icon = match recipe.and_then(|r| r.item) {
            Some(item) => Some(crate::items::get(item).icon.to_string()),
            None => content::classic_spell(lesson.spell).map(|s| format!(r"Interface\Icons\{}", if s.icon == "Temp" { "INV_Misc_Book_09" } else { s.icon.as_str() })),
        };
        self.tree.set_text("ClassTrainerSkillName", &lesson_name(lesson));
        self.tree.set_text("ClassTrainerSubSkillName", "");
        self.tree.set_button_texture("ClassTrainerSkillIcon", icon.as_deref());
        self.tree.set_text("ClassTrainerSkillRequirements", &if needs.is_empty() { String::new() } else { format!("Requires: {}", needs.join(", ")) });
        self.tree.set_money("ClassTrainerDetailMoneyFrame", lesson.cost, true, false);
        let step_text = lesson.step.map(|(skill, step)| format!("Teaches you {}: you can take it up to rank {}.", skill.name(), step * crate::skills::PROFESSION_STEP));
        self.tree.set_text("ClassTrainerSkillDescription", &step_text.unwrap_or_else(|| content::classic_spell(lesson.spell).map_or("", |s| s.text.as_str()).to_string()));
        for part in ["ClassTrainerSkillName", "ClassTrainerSkillIcon", "ClassTrainerSkillRequirements", "ClassTrainerSkillDescription", "ClassTrainerDetailMoneyFrame", "ClassTrainerCostLabel"] {
            self.tree.show(part, true);
        }
        let available = rows.iter().any(|&(i, standing)| Some(i) == selected && standing == Standing::Available);
        self.tree.set_enabled("ClassTrainerTrainButton", available && self.inv.money >= lesson.cost);
    }

    /// Train: the recipe is paid for and known.
    fn train_lesson(&mut self) {
        let Some((_, def)) = self.trainer.who else { return };
        let c = content::get();
        let Some(lesson) = self.trainer.selected.and_then(|s| c.lessons.get(&def.id)?.get(s)) else { return };
        if !self.trainer_rows().iter().any(|&(i, standing)| Some(i) == self.trainer.selected && standing == Standing::Available) {
            return;
        }
        if self.inv.money < lesson.cost {
            return self.error(&self.string("ERR_NOT_ENOUGH_MONEY"));
        }
        self.inv.money -= lesson.cost;
        let name = lesson_name(lesson);
        let text = if let Some((skill, step)) = lesson.step {
            self.skills.teach(skill, step);
            self.fill_skills();
            self.string("ERR_LEARN_ABILITY_S").replacen("%s", &name, 1)
        } else {
            self.recipes.learn(lesson.spell);
            self.string("ERR_LEARN_RECIPE_S").replacen("%s", &name, 1)
        };
        self.notice(&text);
        self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
        self.trainer.selected = self.trainer_rows().first().map(|r| r.0);
        self.refresh();
    }

    /// The wheel over the window scrolls the list by lines.
    pub(super) fn trainer_wheel(&mut self, lines: f32) -> bool {
        if !self.trainer_open() {
            return false;
        }
        self.trainer.offset = (self.trainer.offset as f32 - lines * 3.0).max(0.0) as usize;
        self.fill_trainer();
        true
    }

    /// The tooltip of the picture of a lesson: the item it makes.
    pub(super) fn trainer_lines(&self, name: &str) -> Option<Vec<TipLine>> {
        if name != "ClassTrainerSkillIcon" {
            return None;
        }
        let (_, def) = self.trainer.who?;
        let lesson = content::get().lessons.get(&def.id)?.get(self.trainer.selected?)?;
        let item = recipes::get(lesson.spell)?.1.item?;
        Some(self.item_lines(crate::items::Stack { item, count: 1, enchant: 0 }))
    }

    /// A click on the window. False when it was none of its buttons.
    pub(super) fn trainer_click(&mut self, name: &str) -> bool {
        match name {
            "ClassTrainerFrameCloseButton" | "ClassTrainerCancelButton" => self.close_trainer(),
            "ClassTrainerTrainButton" => self.train_lesson(),
            _ => {
                let Some(n) = name.strip_prefix("ClassTrainerSkill").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=ROWS).contains(n)) else { return false };
                if let Some(&(l, _)) = self.trainer_rows().get(self.trainer.offset + n - 1) {
                    self.trainer.selected = Some(l);
                }
            }
        }
        self.fill_trainer();
        true
    }
}
