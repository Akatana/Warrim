//! The trade skill window: the original's `TradeSkillFrame` (the `Blizzard_TradeSkillUI` addon) filled with the recipes a character knows of one trade. Its Lua
//! (`TradeSkillFrame_Update`, `TradeSkillFrame_SetSelection`) is ported here: the list of eight lines under headings that fold, each coloured by how hard it is for
//! the rank, the picture and reagents of the one selected with what is in the pack, the number to make, and Create and Create All, which ask the game to
//! begin the work (`take_craft`).

use super::*;
use crate::recipes;
use crate::skills::Skill;

pub const TRADE: &str = "TradeSkillFrame";
const ROWS: usize = 8;
const REAGENTS: usize = 8;
const LIST: &str = "TradeSkillListScrollFrame";

/// What the window keeps: the trade it shows, the recipe selected (a place in the content), the first line shown, the headings folded and the number to make.
pub struct Trade {
    skill: Option<Skill>,
    selected: Option<usize>,
    offset: usize,
    folded: Vec<&'static str>,
    count: u32,
}

impl Default for Trade {
    fn default() -> Trade {
        Trade { skill: None, selected: None, offset: 0, folded: Vec::new(), count: 1 }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Header(&'static str),
    Recipe(usize),
}

/// Load the window and take away what this game has nothing for (the filters).
pub fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate(TRADE)?;
    tree.show(TRADE, false);
    for part in ["TradeSkillInvSlotDropDown", "TradeSkillSubClassDropDown"] {
        tree.show(part, false);
    }
    Ok(())
}

impl Panels {
    pub fn trade_open(&self) -> bool {
        self.tree.is_name_shown(TRADE)
    }

    /// The recipes of the trade that the character knows and that make an item, under their headings.
    fn trade_rows(&self) -> Vec<Row> {
        let Some(skill) = self.trade.skill else { return Vec::new() };
        let c = content::get();
        let mut rows = Vec::new();
        let mut last = "";
        for i in recipes::of(skill) {
            let r = &c.recipes[i];
            if !self.recipes.knows(r) {
                continue;
            }
            let head = recipes::category(r);
            if head != last {
                rows.push(Row::Header(head));
                last = head;
            }
            if !self.trade.folded.contains(&head) {
                rows.push(Row::Recipe(i));
            }
        }
        rows
    }

    /// Open the window on a trade (`TradeSkillFrame_Show`): a left window, like the character sheet.
    pub fn open_trade(&mut self, skill: Skill) {
        if !self.skills.knows(skill) {
            return;
        }
        self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
        self.close_loot();
        self.close_book();
        self.close_gossip();
        self.close_merchant();
        self.close_quest();
        self.hide_log();
        if self.trade.skill != Some(skill) {
            self.trade = Trade { skill: Some(skill), ..Trade::default() };
        }
        self.trade.count = 1;
        self.tree.show(TRADE, true);
        self.events.push(Sfx::Named("igCharacterInfoOpen"));
        self.trade_first();
        self.fill_trade();
    }

    pub fn close_trade(&mut self) {
        if self.trade_open() {
            self.tree.show(TRADE, false);
            self.events.push(Sfx::Named("igCharacterInfoClose"));
            self.after_close();
        }
    }

    /// `GetFirstTradeSkill`: the first recipe is selected unless one still is.
    fn trade_first(&mut self) {
        let rows = self.trade_rows();
        if !self.trade.selected.is_some_and(|s| rows.contains(&Row::Recipe(s))) {
            self.trade.selected = rows.iter().find_map(|r| if let Row::Recipe(i) = r { Some(*i) } else { None });
        }
    }

    /// For the headless picture: a blacksmith of rank 60 who knows the first recipes of the trade and carries the metal for some (`--panel trade`).
    pub fn trade_preview(&mut self, skill: Skill) {
        self.skill_list.preview(skill);
        self.skills.set(skill, 60);
        for &i in recipes::of(skill).iter().take(40) {
            self.recipes.learn(content::get().recipes[i].spell);
        }
        for (id, count) in [("i2840", 20), ("i2841", 8), ("i2589", 10), ("i2835", 6), ("i10940", 6), ("i10938", 6)] {
            if let Some(item) = crate::items::all().iter().position(|i| i.id == id) {
                self.inv.add(crate::items::Stack { item, count, enchant: 0 });
            }
        }
        self.open_trade(skill);
    }

    /// The recipe the craft button asked for since the last call: (its place in the content, how many times).
    pub fn take_craft(&mut self) -> Option<(usize, u32)> {
        self.craft_request.take()
    }

    /// The tools and the station a recipe asks for, as the window names them.
    fn requirements(&self, r: &content::Recipe) -> String {
        let mut names: Vec<String> = r.tools.iter().map(|&t| crate::items::get(t).name.to_string()).collect();
        if r.focus != 0 {
            names.push(recipes::station_name(r.focus).to_string());
        }
        names.join(", ")
    }

    /// `TradeSkillFrame_Update` and `_SetSelection`: the list, the rank, the picture and reagents of the recipe selected, and the buttons.
    pub(super) fn fill_trade(&mut self) {
        if !self.trade_open() {
            return;
        }
        let Some(skill) = self.trade.skill else { return };
        let rows = self.trade_rows();
        self.trade.offset = self.trade.offset.min(rows.len().saturating_sub(ROWS));
        let rank = self.skills.rank(skill);
        let cap = self.skills.cap(skill);
        let c = content::get();
        self.tree.set_text("TradeSkillFrameTitleText", skill.name());
        self.tree.set_bar("TradeSkillRankFrame", rank as f32 / cap as f32, Some([0.0, 0.0, 1.0, 0.5]));
        self.tree.set_region_tint("TradeSkillRankFrameBackground", [0.0, 0.0, 0.75, 0.5]);
        self.tree.set_text("TradeSkillRankFrameSkillName", skill.name());
        self.tree.set_text("TradeSkillRankFrameSkillRank", &format!("{rank}/{cap}"));
        self.tree.set_region_point("TradeSkillRankFrameSkillName", Point::Left, Some("TradeSkillRankFrame"), Some(Point::Left), 6.0, 1.0);
        self.tree.set_region_point("TradeSkillRankFrameSkillRank", Point::Left, Some("TradeSkillRankFrameSkillName"), Some(Point::Right), 13.0, 0.0);
        self.tree.set_button_texture("TradeSkillCollapseAllButton", Some(if self.trade.folded.is_empty() { r"Interface\Buttons\UI-MinusButton-Up" } else { r"Interface\Buttons\UI-PlusButton-Up" }));
        self.tree.show("TradeSkillHighlightFrame", false);
        for i in 1..=ROWS {
            let button = format!("TradeSkillSkill{i}");
            let Some(&row) = rows.get(self.trade.offset + i - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            self.tree.show(&button, true);
            match row {
                Row::Header(name) => {
                    self.tree.set_text(&format!("{button}Text"), name);
                    self.tree.set_text_color(&format!("{button}Text"), GOLD);
                    self.tree.set_button_texture(&button, Some(if self.trade.folded.contains(&name) { r"Interface\Buttons\UI-PlusButton-Up" } else { r"Interface\Buttons\UI-MinusButton-Up" }));
                }
                Row::Recipe(r) => {
                    let recipe = &c.recipes[r];
                    let have = recipes::makeable(&self.inv, recipe);
                    let text = if have == 0 { format!(" {}", recipe.name) } else { format!(" {} [{have}]", recipe.name) };
                    self.tree.set_text(&format!("{button}Text"), &text);
                    self.tree.set_text_color(&format!("{button}Text"), recipes::difficulty(recipe, rank).color());
                    self.tree.set_button_texture(&button, None);
                    if self.trade.selected == Some(r) {
                        self.tree.set_point("TradeSkillHighlightFrame", Point::TopLeft, Some(&button), Some(Point::TopLeft), 0.0, 0.0);
                        self.tree.set_region_tint("TradeSkillHighlight", recipes::difficulty(recipe, rank).color());
                        self.tree.show("TradeSkillHighlightFrame", true);
                    }
                }
            }
        }
        // The bar of the list is there only when there are more lines than fit.
        let more = rows.len().saturating_sub(ROWS);
        let bar = "TradeSkillListScrollFrameScrollBar";
        self.tree.show(bar, more > 0);
        let screen = self.tree.screen();
        if let (true, Some(track)) = (more > 0, self.tree.rect_of(bar, screen)) {
            let travel = (track.h - 16.0).max(0.0);
            self.tree.set_region_point(&format!("{bar}ThumbTexture"), Point::TopLeft, Some(bar), Some(Point::TopLeft), 0.0, -travel * self.trade.offset as f32 / more as f32);
        }
        self.fill_trade_detail();
    }

    /// The recipe selected: its name and picture, what it asks for, its reagents with what is carried, and whether it can be made.
    fn fill_trade_detail(&mut self) {
        let c = content::get();
        let Some(r) = self.trade.selected.map(|i| &c.recipes[i]) else {
            for part in ["TradeSkillSkillName", "TradeSkillSkillIcon", "TradeSkillRequirementLabel", "TradeSkillSkillIconCount"] {
                self.tree.show(part, false);
            }
            self.tree.set_text("TradeSkillRequirementText", "");
            for n in 1..=REAGENTS {
                self.tree.show(&format!("TradeSkillReagent{n}"), false);
            }
            for button in ["TradeSkillCreateButton", "TradeSkillCreateAllButton"] {
                self.tree.set_enabled(button, false);
            }
            return;
        };
        // What it makes shows by its picture; an enchantment by the picture of its spell.
        let icon = r.item.map_or_else(|| format!(r"Interface\Icons\{}", content::classic_spell(r.spell).map_or("INV_Misc_QuestionMark", |s| s.icon.as_str())), |i| crate::items::get(i).icon.to_string());
        self.tree.set_text("TradeSkillSkillName", &r.name);
        self.tree.show("TradeSkillSkillName", true);
        self.tree.show("TradeSkillSkillIcon", true);
        self.tree.set_button_texture("TradeSkillSkillIcon", Some(&icon));
        let made = if r.count.0 == r.count.1 { r.count.0.to_string() } else { format!("{}-{}", r.count.0, r.count.1) };
        self.tree.set_text("TradeSkillSkillIconCount", &made);
        self.tree.set_region_hidden("TradeSkillSkillIconCount", r.count.1 <= 1);
        let needs = self.requirements(r);
        self.tree.set_region_hidden("TradeSkillRequirementLabel", needs.is_empty());
        self.tree.set_text("TradeSkillRequirementText", &needs);
        let mut creatable = true;
        for n in 1..=REAGENTS {
            let button = format!("TradeSkillReagent{n}");
            let Some(&(reagent, need)) = r.reagents.get(n - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            let (it, have) = (crate::items::get(reagent), self.inv.count(reagent));
            self.tree.show(&button, true);
            self.tree.set_texture(&format!("{button}IconTexture"), Some(it.icon));
            self.tree.set_text(&format!("{button}Name"), it.name);
            let enough = have >= need;
            creatable &= enough;
            self.tree.set_region_tint(&format!("{button}IconTexture"), if enough { WHITE } else { [0.5, 0.5, 0.5, 1.0] });
            self.tree.set_text_color(&format!("{button}Name"), if enough { WHITE } else { GREY });
            self.tree.set_text(&format!("{button}Count"), &format!("{} /{need}", if have >= 100 { "*".to_string() } else { have.to_string() }));
        }
        for button in ["TradeSkillCreateButton", "TradeSkillCreateAllButton"] {
            self.tree.set_enabled(button, creatable);
        }
    }

    /// The number to make, in the box beside the buttons (the engine has no edit boxes; the plus and minus buttons change it).
    pub(super) fn draw_trade(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        let Some(r) = self.trade_open().then(|| self.tree.rect_of("TradeSkillInputBox", screen)).flatten() else { return };
        let (mount, s) = (self.mount.clone(), screen.1 as f32 / ui_height());
        let text = self.trade.count.to_string();
        self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlight", &text, ((r.x + r.w / 2.0) * s, screen.1 as f32 - (r.y + r.h) * s + 3.0 * s), 0.5, WHITE);
    }

    /// The wheel over the window scrolls the list by lines.
    pub(super) fn trade_wheel(&mut self, lines: f32) -> bool {
        if !self.trade_open() {
            return false;
        }
        self.trade.offset = (self.trade.offset as f32 - lines * 3.0).max(0.0) as usize;
        self.fill_trade();
        true
    }

    /// The tooltip of the picture of the recipe and of its reagents.
    pub(super) fn trade_lines(&self, name: &str) -> Option<Vec<TipLine>> {
        let c = content::get();
        let r = &c.recipes[self.trade.selected?];
        if name == "TradeSkillSkillIcon" {
            return match r.item {
                Some(item) => Some(self.item_lines(crate::items::Stack { item, count: 1, enchant: 0 })),
                None => {
                    let enchant = content::get().enchants.get(&r.enchant).map_or("", |e| e.name.as_str());
                    Some(vec![TipLine::new(r.name.clone(), WHITE), TipLine::new(enchant.to_string(), GREEN)])
                }
            };
        }
        let n = name.strip_prefix("TradeSkillReagent")?.parse::<usize>().ok()?;
        let &(reagent, count) = r.reagents.get(n.checked_sub(1)?)?;
        Some(self.item_lines(crate::items::Stack { item: reagent, count, enchant: 0 }))
    }

    /// A click on the window. False when it was none of its buttons.
    pub(super) fn trade_click(&mut self, name: &str) -> bool {
        match name {
            "TradeSkillFrameCloseButton" | "TradeSkillCancelButton" => self.close_trade(),
            "TradeSkillCreateButton" | "TradeSkillCreateAllButton" => {
                let Some(r) = self.trade.selected else { return true };
                let have = recipes::makeable(&self.inv, &content::get().recipes[r]);
                let times = if name == "TradeSkillCreateAllButton" { have } else { self.trade.count.min(have.max(1)) };
                self.trade.count = times.max(1);
                self.craft_request = Some((r, times.max(1)));
            }
            "TradeSkillIncrementButton" => self.trade.count = (self.trade.count + 1).min(100),
            "TradeSkillDecrementButton" => self.trade.count = self.trade.count.saturating_sub(1).max(1),
            "TradeSkillCollapseAllButton" => {
                if self.trade.folded.is_empty() {
                    let heads: Vec<&'static str> = self.trade_rows().into_iter().filter_map(|r| if let Row::Header(h) = r { Some(h) } else { None }).collect();
                    self.trade.folded = heads;
                    self.trade.offset = 0;
                } else {
                    self.trade.folded.clear();
                }
            }
            _ => {
                let Some(n) = name.strip_prefix("TradeSkillSkill").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=ROWS).contains(n)) else { return false };
                match self.trade_rows().get(self.trade.offset + n - 1) {
                    Some(&Row::Header(head)) => {
                        match self.trade.folded.iter().position(|h| *h == head) {
                            Some(at) => drop(self.trade.folded.remove(at)),
                            None => self.trade.folded.push(head),
                        }
                    }
                    Some(&Row::Recipe(r)) => {
                        self.trade.selected = Some(r);
                        self.trade.count = 1;
                    }
                    None => {}
                }
            }
        }
        self.fill_trade();
        true
    }
}
