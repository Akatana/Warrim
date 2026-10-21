//! The spellbook (`SpellBookFrame`, key P) and the skills tab of the character sheet (`SkillFrame`): the original's frames, filled with the
//! spells the character knows, twelve to a page, and the skills with their ranks, in lists with headings that fold. Clicking a spell
//! casts it, as in the original.

use super::*;
use crate::actionbar::{Action, BASIC};
use crate::content::{Cost, Effect};
use crate::skills::{self, Group, Skill};

pub const SPELLBOOK: &str = "SpellBookFrame";
const SPELL_BUTTONS: usize = 12;
const SKILL_ROWS: usize = 12;
const SKILLS: &str = "SkillFrame";
const GROUPS: [Group; 4] = [Group::Class, Group::Weapon, Group::Armor, Group::General];

/// The cast bar's state, as `CastingBarFrame.lua` keeps it: filling (yellow, with its spark), done (green, flashing, fading out) or
/// failed (red with the reason, held a second, fading out).
#[derive(Default)]
pub struct CastBar {
    /// Seconds since the game began to ask (the bar's own clock), when this cast began and when it ends.
    clock: f32,
    start: f32,
    end: f32,
    casting: bool,
    flash: bool,
    fade_out: bool,
    hold_until: f32,
    alpha: f32,
    flash_alpha: f32,
    shown: bool,
}

/// A line of the skills list.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Header(Group),
    Skill(Skill),
}

/// What the skills tab keeps: its scroll position in rows, the skill selected, and the headings folded.
pub struct SkillList {
    offset: usize,
    selected: Option<Skill>,
    folded: [bool; 4],
    /// Whether the tab shows (and not the doll).
    pub shown: bool,
}

impl Default for SkillList {
    fn default() -> SkillList {
        SkillList { offset: 0, selected: None, folded: [false; 4], shown: false }
    }
}

/// The rows of the list: each heading, and under it its skills unless it is folded.
fn rows(folded: &[bool; 4]) -> Vec<Row> {
    let mut out = Vec::new();
    for (g, group) in GROUPS.iter().enumerate() {
        out.push(Row::Header(*group));
        if !folded[g] {
            // In the order of the alphabet, as the original lists them.
            let mut of: Vec<Skill> = skills::ALL.iter().copied().filter(|s| s.group() == *group).collect();
            of.sort_by_key(|s| s.name());
            out.extend(of.into_iter().map(Row::Skill));
        }
    }
    out
}

/// `PanelTemplates_SelectTab` and `DeselectTab`: the selected tab shows its raised picture, in white and out of reach of clicks.
fn select_tab(tree: &mut Tree, tab: &str, selected: bool) {
    for part in ["Left", "Middle", "Right"] {
        tree.set_region_hidden(&format!("{tab}{part}"), selected);
        tree.set_region_hidden(&format!("{tab}{part}Disabled"), !selected);
    }
    tree.set_enabled(tab, !selected);
    tree.set_text_color(&format!("{tab}Text"), if selected { WHITE } else { GOLD });
}

/// Which button of the spellbook shows the spell at place `slot` of a page: the frame numbers them down the left column and then
/// down the right, but names the buttons across the rows.
fn spell_button(slot: usize) -> usize {
    if slot < 6 { 2 * slot + 1 } else { 2 * (slot - 6) + 2 }
}

/// The place on the page of the button named `SpellButton{n}`.
pub(super) fn button_slot(n: usize) -> usize {
    if n % 2 == 1 { (n - 1) / 2 } else { 6 + (n - 2) / 2 }
}

/// Load the two frames and take away what this game has nothing for.
pub fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate("CastingBarFrame")?;
    tree.show("CastingBarFrame", false);
    // Above the health, mana and stamina bars of this game's action bar (the original's sits just above its own).
    tree.set_point("CastingBarFrame", Point::Bottom, Some("UIParent"), Some(Point::Bottom), 0.0, 100.0);
    // The breath bar: the original's first mirror timer, blue.
    tree.instantiate("MirrorTimer1")?;
    tree.show("MirrorTimer1", false);
    // (its label is drawn by `draw_overlay`: the engine draws a frame's own text under its children, so the bar would cover it.)
    tree.set_region_hidden("MirrorTimer1Text", true);
    tree.instantiate(SPELLBOOK)?;
    tree.show(SPELLBOOK, false);
    // One book of spells (no pets, no tabs), and the lines of one list.
    for tab in 1..=3 {
        tree.show(&format!("SpellBookFrameTabButton{tab}"), false);
    }
    tree.show("SpellBookTabFlashFrame", false);
    for i in 1..=SPELL_BUTTONS {
        let button = format!("SpellButton{i}");
        for region in ["Cooldown", "AutoCast"] {
            tree.show(&format!("{button}{region}"), false);
        }
        tree.set_region_hidden(&format!("{button}AutoCastable"), true);
    }
    // (`SkillFrame` was loaded with the character sheet, whose child it is.)
    tree.show(SKILLS, false);
    // No learning, forgetting or raising of skills with points: ranks come from use. No hit on the list's own scroll bar knob, which
    // does not know the rows.
    for part in ["SkillDetailStatusBarLearnSkillButton", "SkillDetailStatusBarLeftArrow", "SkillDetailStatusBarRightArrow", "SkillDetailStatusBarUnlearnButton", "SkillDetailCostText", "SkillDetailScrollFrameScrollBar"] {
        tree.show(part, false);
    }
    for i in 1..=SKILL_ROWS {
        tree.set_region_hidden(&format!("SkillRankFrame{i}FillBar"), true);
    }
    // The skills tab beside the character's.
    tree.show("CharacterFrameTab4", true);
    tree.set_point("CharacterFrameTab4", Point::Left, Some("CharacterFrameTab1"), Some(Point::Right), -16.0, 0.0);
    // The button that folds every heading says "All", on a tab as wide as its text.
    let all = tree.string("ALL");
    tree.set_text("SkillFrameCollapseAllButtonText", &all);
    tree.set_size("SkillFrameExpandButtonFrame", Some(80.0), None);
    tree.set_size("SkillFrameCollapseAllButton", Some(66.0), None);
    let name = tree.string("SKILLS");
    tree.set_text("CharacterFrameTab4Text", &name);
    Ok(())
}

impl Panels {
    // ---- the cast bar (`CastingBarFrame_OnEvent` and `_OnUpdate`)

    /// `SPELLCAST_START`: the bar begins to fill, in orange, with the name of the spell.
    pub fn cast_start(&mut self, name: &str, seconds: f32) {
        let b = &mut self.cast_bar;
        (b.start, b.end, b.casting, b.fade_out, b.flash, b.hold_until, b.alpha, b.shown) = (b.clock, b.clock + seconds, true, false, false, 0.0, 1.0, true);
        self.tree.set_bar("CastingBarFrame", 0.0, Some([1.0, 0.7, 0.0, 1.0]));
        self.tree.set_text("CastingBarText", name);
        self.tree.set_text_color("CastingBarText", WHITE);
        self.tree.set_region_hidden("CastingBarSpark", false);
        self.tree.set_region_hidden("CastingBarFlash", true);
        self.tree.set_alpha("CastingBarFrame", 1.0);
        self.tree.show("CastingBarFrame", true);
    }

    /// `SPELLCAST_STOP`: full and green, with a flash, then it fades.
    pub fn cast_stop(&mut self) {
        if !self.cast_bar.shown {
            return;
        }
        let b = &mut self.cast_bar;
        (b.casting, b.flash, b.fade_out, b.flash_alpha) = (false, true, true, 0.0);
        self.tree.set_bar("CastingBarFrame", 1.0, Some([0.0, 1.0, 0.0, 1.0]));
        self.tree.set_region_hidden("CastingBarSpark", true);
        self.tree.set_region_hidden("CastingBarFlash", false);
        self.tree.set_region_tint("CastingBarFlash", [1.0, 1.0, 1.0, 0.0]);
    }

    /// `SPELLCAST_FAILED` and `_INTERRUPTED`: full and red with the reason, held for a second, then it fades.
    pub fn cast_failed(&mut self, why: &str) {
        if !self.cast_bar.shown {
            return;
        }
        let b = &mut self.cast_bar;
        (b.casting, b.fade_out, b.hold_until) = (false, true, b.clock + 1.0);
        self.tree.set_bar("CastingBarFrame", 1.0, Some([1.0, 0.0, 0.0, 1.0]));
        self.tree.set_region_hidden("CastingBarSpark", true);
        self.tree.set_text("CastingBarText", why);
    }

    /// Whether the bar shows (filling, or fading out).
    pub(super) fn cast_bar_visible(&self) -> bool {
        self.cast_bar.shown
    }

    pub fn update_cast_bar(&mut self, dt: f32) {
        let b = &mut self.cast_bar;
        b.clock += dt;
        if !b.shown {
            return;
        }
        // The original steps by frames (60 to the second).
        let steps = dt * 60.0;
        if b.casting {
            let frac = ((b.clock - b.start) / (b.end - b.start).max(0.001)).clamp(0.0, 1.0);
            self.tree.set_bar("CastingBarFrame", frac, None);
            self.tree.set_region_hidden("CastingBarFlash", true);
            self.tree.set_region_point("CastingBarSpark", Point::Center, Some("CastingBarFrame"), Some(Point::Left), frac * 195.0, 2.0);
        } else if b.clock < b.hold_until {
        } else if b.flash {
            b.flash_alpha += 0.2 * steps;
            if b.flash_alpha >= 1.0 {
                (b.flash_alpha, b.flash) = (1.0, false);
            }
            self.tree.set_region_tint("CastingBarFlash", [1.0, 1.0, 1.0, b.flash_alpha]);
        } else if b.fade_out {
            b.alpha -= 0.05 * steps;
            if b.alpha > 0.0 {
                self.tree.set_alpha("CastingBarFrame", b.alpha);
            } else {
                (b.fade_out, b.shown) = (false, false);
                self.tree.show("CastingBarFrame", false);
            }
        }
    }

    /// The breath left and the most there is, in seconds (none: the bar goes away), as the player reports it each frame.
    pub fn set_breath(&mut self, breath: Option<(f32, f32)>) {
        if let Some((left, max)) = breath {
            if !self.breath_shown {
                self.tree.show("MirrorTimer1", true);
            }
            self.tree.set_bar("MirrorTimer1StatusBar", left / max, Some([0.0, 0.5, 1.0, 1.0]));
        } else if self.breath_shown {
            self.tree.show("MirrorTimer1", false);
        }
        self.breath_shown = breath.is_some();
    }

    // ---- the spellbook

    pub fn spellbook_open(&self) -> bool {
        self.tree.is_name_shown(SPELLBOOK)
    }

    pub fn toggle_spellbook(&mut self) {
        let on = !self.spellbook_open();
        if on {
            self.tree.show(CHARACTER, false);
            self.close_loot();
            self.close_gossip();
            self.close_merchant();
            self.close_quest();
            self.hide_log();
        }
        self.tree.show(SPELLBOOK, on);
        self.events.push(Sfx::Named(if on { "igSpellBookOpen" } else { "igSpellBookClose" }));
        self.fill_spellbook();
        self.after_close();
    }

    /// The spell a click on the book asked to cast, if there was one.
    pub fn take_spell_click(&mut self) -> Option<usize> {
        self.spell_click.take()
    }

    /// The action being dragged out of the spellbook, and, when the mouse button comes up, the one to put down.
    pub fn dragging(&self) -> Option<Action> {
        self.drag
    }

    pub fn take_drag(&mut self) -> Option<Action> {
        self.drag.take()
    }

    /// The book lists the basic actions, then the spells known.
    fn book_len(&self) -> usize {
        BASIC.len() + self.known.len()
    }

    fn book_action(&self, index: usize) -> Option<Action> {
        BASIC.get(index).copied().or_else(|| self.known.get(index.checked_sub(BASIC.len())?).map(|&s| Action::Spell(s)))
    }

    /// What the book's button `name` (`SpellButton{n}`) shows on the page now.
    pub(super) fn book_entry_of(&self, name: &str) -> Option<Action> {
        let n = name.strip_prefix("SpellButton")?.parse::<usize>().ok().filter(|n| (1..=SPELL_BUTTONS).contains(n))?;
        self.book_action(self.book_page * SPELL_BUTTONS + button_slot(n))
    }

    fn book_pages(&self) -> usize {
        self.book_len().div_ceil(SPELL_BUTTONS).max(1)
    }

    /// `SpellBookFrame_Update`: the spells of the page, their icons and names, and the page buttons.
    pub(super) fn fill_spellbook(&mut self) {
        if !self.spellbook_open() {
            return;
        }
        self.book_page = self.book_page.min(self.book_pages() - 1);
        for slot in 0..SPELL_BUTTONS {
            let button = format!("SpellButton{}", spell_button(slot));
            let action = self.book_action(self.book_page * SPELL_BUTTONS + slot);
            self.tree.show(&button, action.is_some());
            let Some(action) = action else { continue };
            let (icon, name, kind) = match (action, action.basic_icon()) {
                (Action::Spell(i), _) => {
                    let def = content::spell(i);
                    (format!(r"Interface\Icons\{}", def.icon), def.name.clone(), def.class.name())
                }
                (a, icon) => (icon.unwrap_or_default().to_string(), a.basic_info().map_or(String::new(), |(n, _)| n.to_string()), "Basic"),
            };
            self.tree.set_texture(&format!("{button}IconTexture"), Some(&icon));
            self.tree.set_region_hidden(&format!("{button}IconTexture"), false);
            self.tree.set_text(&format!("{button}SpellName"), &name);
            self.tree.set_region_hidden(&format!("{button}SpellName"), false);
            self.tree.set_text(&format!("{button}SubSpellName"), kind);
            self.tree.set_region_hidden(&format!("{button}SubSpellName"), false);
        }
        self.tree.set_text("SpellBookPageText", &format!("Page {}", self.book_page + 1));
        self.tree.set_enabled("SpellBookPrevPageButton", self.book_page > 0);
        self.tree.set_enabled("SpellBookNextPageButton", self.book_page + 1 < self.book_pages());
    }

    /// The tooltip of a spellbook entry: a spell's, or a basic action's name and what it does.
    pub(super) fn action_lines(&self, action: Action) -> Vec<TipLine> {
        match (action, action.basic_info()) {
            (Action::Spell(i), _) => self.spell_lines(i),
            (_, Some((name, what))) => vec![TipLine::new(name, WHITE), TipLine::new(what, GOLD)],
            _ => Vec::new(),
        }
    }

    /// What the numbers of a spell (spells.ron, written for level 1) are multiplied by: the rank of the class skill and the
    /// character level, which grows like its health so a heal or a bolt keeps its share of it.
    pub fn spell_power(&self, class: Skill) -> f32 {
        self.skills.damage_factor(class) * levels::spell_scale(self.progress.level)
    }

    /// What the worn items add to what a spell does: the damage of its school (the healing for a heal) times what its cast time gives it (CMaNGOS: 1.5 to 3.5
    /// seconds out of 3.5, a third of that for what hits an area), less for spells below level 20.
    pub fn spell_bonus(&self, spell: usize) -> f32 {
        let def = content::spell(spell);
        let (cast, school, level) = content::classic_spell(def.spell).map_or((0, 0, 20), |c| (c.cast, c.school, c.level));
        let power = match def.effect {
            Effect::Heal { .. } => self.stats.healing,
            _ => self.stats.spell_damage[(school as usize).min(6)],
        };
        let area = if matches!(def.effect, Effect::Area { .. }) { 1.0 / 3.0 } else { 1.0 };
        let penalty = if level > 0 && level < 20 { 1.0 - (20.0 - level as f32) * 0.0375 } else { 1.0 };
        power as f32 * (cast.clamp(1500, 3500) as f32 / 3500.0) * area * penalty
    }

    /// The chance to land a critical hit with a spell: by its school (the physical ones crit as blows do).
    pub fn spell_crit(&self, spell: usize) -> f32 {
        match content::classic_spell(content::spell(spell).spell).map_or(0, |c| c.school) {
            0 => self.stats.crit,
            school => self.stats.spell_crit[(school as usize).min(6)],
        }
    }

    /// What a spell costs at the character level: mana grows like its numbers (stamina does not).
    pub fn spell_cost(&self, cost: Cost) -> Cost {
        match cost {
            Cost::Mana(n) => Cost::Mana(n * levels::spell_scale(self.progress.level)),
            stamina => stamina,
        }
    }

    /// The tooltip of a spell: its cost and range, its cooldown, what it does at the skill the character has, and what it is.
    pub(super) fn spell_lines(&self, spell: usize) -> Vec<TipLine> {
        let def = content::spell(spell);
        let power = self.spell_power(def.class);
        let bonus = self.spell_bonus(spell);
        let cost = match self.spell_cost(def.cost) {
            Cost::Mana(n) => format!("{n:.0} Mana"),
            Cost::Stamina(n) => format!("{n:.0} Stamina"),
        };
        let (range, effect) = match def.effect {
            Effect::Damage { min, max, range } => (format!("{range:.0} yd range"), format!("Hurts the target for {:.0} to {:.0}.", min * power + bonus, max * power + bonus)),
            Effect::Area { min, max, radius } => (format!("{radius:.0} yd around you"), format!("Hurts every enemy near you for {:.0} to {:.0}.", min * power + bonus, max * power + bonus)),
            Effect::Heal { min, max } => ("Self".to_string(), format!("Heals you for {:.0} to {:.0}.", min * power + bonus, max * power + bonus)),
        };
        vec![
            TipLine::new(def.name.clone(), WHITE),
            TipLine::double(cost, range, WHITE, WHITE),
            TipLine::double("Instant cast", format!("{:.0} sec cooldown", def.cooldown), WHITE, WHITE),
            TipLine::new(effect, GOLD),
            TipLine::new(def.description.clone(), GOLD),
            TipLine::new(format!("{} skill: {}", def.class.name(), self.skills.rank(def.class)), GREY),
        ]
    }

    // ---- the skills tab

    /// Show the skills (`true`) or the doll on the character sheet.
    pub fn show_skills_tab(&mut self, skills: bool) {
        self.skill_list.shown = skills;
        self.tree.show("PaperDollFrame", !skills);
        self.tree.show(SKILLS, skills);
        select_tab(&mut self.tree, "CharacterFrameTab1", !skills);
        select_tab(&mut self.tree, "CharacterFrameTab4", skills);
        if skills {
            self.fill_skills();
        }
    }

    /// `SkillFrame_UpdateSkills`: twelve lines of the list from the scroll position, and the detail of the selected skill.
    pub(super) fn fill_skills(&mut self) {
        if !self.skill_list.shown {
            return;
        }
        let rows = rows(&self.skill_list.folded);
        self.skill_list.offset = self.skill_list.offset.min(rows.len().saturating_sub(SKILL_ROWS));
        for i in 1..=SKILL_ROWS {
            let (label, bar) = (format!("SkillTypeLabel{i}"), format!("SkillRankFrame{i}"));
            match rows.get(self.skill_list.offset + i - 1) {
                Some(&Row::Header(group)) => {
                    let g = GROUPS.iter().position(|&x| x == group).unwrap_or(0);
                    self.tree.show(&label, true);
                    self.tree.show(&bar, false);
                    self.tree.set_text(&format!("{label}Text"), group.name());
                    self.tree.set_texture(&format!("{label}NormalTexture"), Some(if self.skill_list.folded[g] { r"Interface\Buttons\UI-PlusButton-Up" } else { r"Interface\Buttons\UI-MinusButton-Up" }));
                }
                Some(&Row::Skill(skill)) => {
                    self.tree.show(&label, false);
                    self.tree.show(&bar, true);
                    self.fill_skill_bar(&bar, skill);
                }
                None => {
                    self.tree.show(&label, false);
                    self.tree.show(&bar, false);
                }
            }
        }
        // The button that folds everything shows a minus while everything is open; the bar's knob stands where the list is, and the
        // bar is there only if there is more than the twelve lines.
        self.tree.set_texture("SkillFrameCollapseAllButtonNormalTexture", Some(if self.skill_list.folded.iter().any(|&f| f) { r"Interface\Buttons\UI-PlusButton-Up" } else { r"Interface\Buttons\UI-MinusButton-Up" }));
        let (bar, more) = ("SkillListScrollFrameScrollBar", rows.len().saturating_sub(SKILL_ROWS));
        self.tree.show(bar, more > 0);
        let screen = self.tree.screen();
        if let (true, Some(track)) = (more > 0, self.tree.rect_of(bar, screen)) {
            let travel = (track.h - 16.0).max(0.0);
            self.tree.set_region_point(&format!("{bar}ThumbTexture"), Point::TopLeft, Some(bar), Some(Point::TopLeft), 0.0, -travel * self.skill_list.offset as f32 / more as f32);
        }
        self.fill_skill_detail();
    }

    /// One bar of the list, or the detail's: the name, the rank out of the top, and the fill.
    fn fill_skill_bar(&mut self, bar: &str, skill: Skill) {
        let rank = self.skills.rank(skill);
        let cap = skill.cap(rank);
        self.tree.set_bar(bar, rank as f32 / cap as f32, Some([0.0, 0.0, 1.0, 0.5]));
        self.tree.set_region_tint(&format!("{bar}Background"), [0.0, 0.0, 0.75, 0.5]);
        self.tree.set_text(&format!("{bar}SkillName"), skill.name());
        self.tree.set_text_color(&format!("{bar}SkillName"), GOLD);
        self.tree.set_text(&format!("{bar}SkillRank"), &format!("{rank}/{cap}"));
        self.tree.set_text_color(&format!("{bar}SkillRank"), WHITE);
        self.tree.set_region_point(&format!("{bar}SkillName"), Point::Left, Some(bar), Some(Point::Left), 6.0, 1.0);
        self.tree.set_region_point(&format!("{bar}SkillRank"), Point::Left, Some(&format!("{bar}SkillName")), Some(Point::Right), 13.0, 0.0);
    }

    /// `SkillDetailFrame_SetStatusBar`: the selected skill's bar and what it is for.
    fn fill_skill_detail(&mut self) {
        let Some(skill) = self.skill_list.selected else {
            self.tree.show("SkillDetailStatusBar", false);
            self.tree.set_text("SkillDetailDescriptionText", "");
            return;
        };
        self.tree.show("SkillDetailStatusBar", true);
        self.fill_skill_bar("SkillDetailStatusBar", skill);
        self.tree.set_text("SkillDetailDescriptionText", &skill.description());
    }

    /// The wheel over the skills list scrolls it by rows. False when the tab is not showing.
    pub(super) fn skills_wheel(&mut self, lines: f32) -> bool {
        if !self.skill_list.shown || !self.tree.is_name_shown(SKILLS) {
            return false;
        }
        self.skill_list.offset = (self.skill_list.offset as f32 - lines * 3.0).max(0.0) as usize;
        self.fill_skills();
        true
    }

    // ---- clicks

    /// `ToggleCharacter(tab)`: the sheet opens at that tab; on the tab it is open at, it closes; on another it goes to it.
    pub fn toggle_character_tab(&mut self, skills: bool) {
        if self.character_open() && self.skill_list.shown != skills {
            self.show_skills_tab(skills);
            self.events.push(Sfx::Named("igCharacterInfoTab"));
        } else {
            self.skill_list.shown = skills;
            self.toggle_character();
        }
    }

    /// A click on one of these windows' buttons. False when it was none of them.
    pub(super) fn magic_click(&mut self, name: &str) -> bool {
        if name.strip_prefix("SpellButton").is_some_and(|n| n.parse::<usize>().is_ok()) {
            if let Some(Action::Spell(spell)) = self.book_entry_of(name) {
                self.spell_click = Some(spell);
            }
            return true;
        }
        match name {
            "SpellBookCloseButton" => self.toggle_spellbook(),
            "SpellBookPrevPageButton" | "SpellBookNextPageButton" => {
                let next = name == "SpellBookNextPageButton";
                self.book_page = if next { (self.book_page + 1).min(self.book_pages() - 1) } else { self.book_page.saturating_sub(1) };
                self.events.push(Sfx::Named("igSpellBokPageTurn"));
                self.fill_spellbook();
            }
            "CharacterFrameTab1" | "CharacterFrameTab4" => {
                self.show_skills_tab(name == "CharacterFrameTab4");
                self.events.push(Sfx::Named("igCharacterInfoTab"));
            }
            "SkillFrameCancelButton" => {
                self.tree.show(CHARACTER, false);
                self.events.push(Sfx::Named("igCharacterInfoClose"));
                self.after_close();
            }
            "SkillFrameCollapseAllButton" => {
                let all = self.skill_list.folded.iter().all(|&f| !f);
                self.skill_list.folded = [all; 4];
                self.fill_skills();
            }
            "SkillListScrollFrameScrollBarScrollUpButton" | "SkillListScrollFrameScrollBarScrollDownButton" => {
                self.skills_wheel(if name.ends_with("UpButton") { 1.0 } else { -1.0 });
            }
            _ => {
                if let Some(i) = name.strip_prefix("SkillTypeLabel").and_then(|n| n.parse::<usize>().ok()) {
                    if let Some(&Row::Header(group)) = rows(&self.skill_list.folded).get(self.skill_list.offset + i - 1) {
                        let g = GROUPS.iter().position(|&x| x == group).unwrap_or(0);
                        self.skill_list.folded[g] = !self.skill_list.folded[g];
                        self.fill_skills();
                    }
                    return true;
                }
                if let Some(i) = name.strip_prefix("SkillRankFrame").and_then(|n| n.strip_suffix("Border")).and_then(|n| n.parse::<usize>().ok()) {
                    if let Some(&Row::Skill(skill)) = rows(&self.skill_list.folded).get(self.skill_list.offset + i - 1) {
                        self.skill_list.selected = Some(skill);
                        self.fill_skills();
                    }
                    return true;
                }
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_of_the_book_run_down_the_left_column_and_then_down_the_right() {
        assert_eq!((0..SPELL_BUTTONS).map(spell_button).collect::<Vec<_>>(), [1, 3, 5, 7, 9, 11, 2, 4, 6, 8, 10, 12]);
        assert!((1..=SPELL_BUTTONS).all(|n| spell_button(button_slot(n)) == n));
    }

    #[test]
    fn the_list_has_a_heading_for_each_group_and_what_is_folded_is_left_out() {
        let all = rows(&[false; 4]);
        assert_eq!(all.len(), 4 + skills::ALL.len());
        assert_eq!(all[0], Row::Header(Group::Class));
        // Folded, a heading has nothing under it; listed, its skills are in the order of the alphabet.
        let folded = rows(&[true, false, false, false]);
        assert_eq!(folded.len(), 4 + skills::ALL.iter().filter(|s| s.group() != Group::Class).count());
        let names: Vec<&str> = folded.iter().filter_map(|r| if let Row::Skill(s) = r { Some(s.name()) } else { None }).collect();
        assert_eq!(names, ["Axes", "Block", "Bows", "Crossbows", "Daggers", "Fist Weapons", "Guns", "Maces", "Polearms", "Spears", "Staves", "Swords", "Thrown", "Two-Handed Axes", "Two-Handed Maces", "Two-Handed Swords", "Unarmed", "Wands", "Cloth", "Leather", "Mail", "Plate Mail", "Diving", "Lockpicking", "Riding", "Stealth", "Swimming"]);
    }
}
