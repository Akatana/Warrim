//! The character sheet and the backpack: the original's frames (`CharacterFrame`, `PaperDollFrame`, `ContainerFrame1`),
//! loaded from its XML by `uixml`, filled with the player's items and stats, with the mouse picking items up and
//! putting them down, tooltips, and the character's 3D model and portrait drawn into the frame.

use crate::audio::Sfx;
use crate::camera::Camera;
use crate::items::{Inventory, ItemDef, Stack, Stats, BACKPACK_SIZE, QUALITY_COLORS, SLOTS, SLOT_BACKGROUNDS, SLOT_BUTTONS};
use crate::content::{self, Action, CreatureDef, Goal};
use crate::quests::{Marker, QuestLog};
use crate::levels::{self, Progress};
use crate::loot::Loot;
use crate::player::Player;
use crate::renderer::{Gpu, Renderer};
use crate::world::World;
use crate::ui::{Quad, Ui, FULL_UV};
use crate::uixml::{ui_height, Point, Registry, TipLine, Tree};
use anyhow::Result;
use glam::Vec3;
use mpq::Mount;
use std::f32::consts::PI;
use std::rc::Rc;

mod magic;
mod keybinds;
mod levelup;
mod map;
mod menu;
mod sleep;
mod talents;
mod trade;
mod trainer;
pub use menu::MenuAction;
use magic::SPELLBOOK;
use talents::TALENTS;
use std::sync::Arc;

const CLASS: &str = "Paladin";
/// Seconds the cast of a mount takes (the original's three).
const MOUNT_SECONDS: f32 = 3.0;
const CHARACTER: &str = "CharacterFrame";
const BACKPACK: &str = "ContainerFrame1";
const BACKPACK_NAME: &str = "Backpack";
const BACKPACK_PORTRAIT: &str = r"Interface\Buttons\Button-Backpack-Up";
/// The backpack's frame is drawn at 0.9 of its size and sits 70 units above the bottom right corner.
const CONTAINER_SCALE: f32 = 0.9;
const CONTAINER_OFFSET_Y: f32 = 70.0;
const CONTAINER_COLUMNS: usize = 4;
/// Texture size of the paper doll and portrait pictures.
const DOLL_SIZE: (u32, u32) = (466, 448);
const PORTRAIT_SIZE: (u32, u32) = (128, 128);
/// Radians the rotate buttons turn the doll by a click.
const DOLL_STEP: f32 = 0.4;

const WHITE: [f32; 4] = [1.0; 4];

pub struct Panels {
    /// Who the character is, for the texts that name it.
    name: String,
    race: crate::character::Race,
    tree: Tree,
    mount: Arc<Mount>,
    pub inv: Inventory,
    /// The first backpack slot the window shows (a multiple of the row length).
    bag_scroll: usize,
    pub stats: Stats,
    /// The character's level and experience.
    pub progress: Progress,
    /// For each place of `pois.ron`, whether the character has discovered it.
    pub found: Vec<bool>,
    /// The world map is open, and the place the player picked on it to travel to.
    map_open: bool,
    map: map::MapState,
    /// The `AreaTable` ids the character has been in: their overlays show on the map.
    pub explored: Vec<u32>,
    travel: Option<(f32, f32, Option<f32>)>,
    /// The skills, and the spells known (indices into the content's spells) in the order they were learned, of which `learned`
    /// are those the game has not yet put on the action bar.
    pub skills: crate::skills::Skills,
    /// The recipes of the trades that were taught.
    pub recipes: crate::recipes::Known,
    pub known: Vec<usize>,
    learned: Vec<usize>,
    /// Spells a talent taught that were given back, for the action bar to drop (`take_forgotten`).
    forgotten: Vec<usize>,
    /// Seconds left before each spell can be cast again.
    pub cooldowns: Vec<f32>,
    /// The spellbook's page, the spell a click on it asked to cast, and the skills tab's list.
    book_page: usize,
    cast_bar: magic::CastBar,
    /// Whether the breath bar (the original's first mirror timer) shows.
    breath_shown: bool,
    spell_click: Option<usize>,
    /// The action picked up from the spellbook, until the mouse button comes up.
    drag: Option<crate::actionbar::Action>,
    skill_list: magic::SkillList,
    /// The talents learned and the tree whose tab shows (`talents.rs`, `panels/talents.rs`).
    talents: crate::talents::Ranks,
    /// The trade skill window's state, and the recipe it asked the game to make (its place in the content, how many times).
    trade: trade::Trade,
    trainer: trainer::Trainer,
    craft_request: Option<(usize, u32)>,
    talent_tab: usize,
    doll: (usize, usize),
    portrait: (usize, usize),
    doll_turn: f32,
    cursor: Option<(f32, f32)>,
    /// Items were worn or taken off since `take_equipment_change`.
    equipment_changed: bool,
    /// The stats changed with no change of what is worn (a spell went off or ran out): the player takes them on.
    stats_dirty: bool,
    /// The corpse being looted (as the game numbers its corpses) and what is left on it.
    loot: Option<(usize, Loot)>,
    /// Loot left behind by a window that closed, for the game to put back on its corpse.
    closed_loot: Option<(usize, Loot)>,
    /// Sounds made by the interface since `take_events`.
    events: Vec<Sfx>,
    /// A red message at the top of the screen and its age in seconds (`UIErrorsFrame`).
    messages: Vec<(String, [f32; 4], f32)>,
    /// The creature being talked to (as the game numbers its creatures), and what it has said instead of its greeting.
    gossip: Option<(usize, &'static CreatureDef, Option<String>)>,
    /// The merchant being traded with and the last thing sold to him (which can be bought back for what it fetched).
    merchant: Option<(usize, &'static CreatureDef)>,
    /// The page of the wares that shows (ten to a page).
    merchant_page: usize,
    /// What using items does now: boosts and regeneration that last, cooldowns, the way home; and whether the character keeps still and out of a fight.
    pub consumed: crate::consume::Consumed,
    /// The spells of creatures that have hold of the character, shown with the buffs: the icon (its file) and the seconds left.
    afflicted: Vec<(String, f32)>,
    calm: bool,
    /// Health and mana that were given at once and are for the player to take (`take_restores`).
    restored: (f32, f32),
    /// A mount item was used (`take_steed`): its display and speed bonus.
    steed: Option<(u32, i32)>,
    /// Whether the character rides (set by the game), and the mount being cast: seconds left, the steed's display and speed bonus.
    mounted: bool,
    mount_cast: Option<(f32, (u32, i32))>,
    /// A lure or a stone to put on the weapon: the item whose cast the game is to start, and whether the cast is over (then using it does it).
    lure: (Option<usize>, bool),
    buyback: Option<(Stack, u32)>,
    /// The quests: what is taken, how far along it is, what is done.
    pub quests: QuestLog,
    /// Where the tracked quest leads now (render space) and its picture; set every frame by `set_quest_targets`.
    quest_targets: Vec<(Vec3, crate::quests::Icon)>,
    /// The lines the talk window shows, in order (what each button means).
    gossip_lines: Vec<Line>,
    /// The quest window: the creature, its kind's definition, the quest and which of the three screens shows.
    questing: Option<(usize, &'static CreatureDef, usize, QuestScreen)>,
    /// How many of each item goal of the active quests were counted last time, to say when it goes up.
    seen: std::collections::HashMap<(usize, usize), u32>,
    /// Quests already announced as complete.
    ready: std::collections::HashSet<usize>,
    /// The quest of the log that is selected (an index into `quests.active`).
    log_selected: usize,
    /// The reward picked on the reward screen (an index into the quest's `choice`).
    reward_choice: Option<usize>,
    /// Levels gained since `take_level_up` (for the effect round the character).
    level_ups: u32,
    /// The question that is asked: abandon this quest? (`StaticPopup1`)
    popup: Option<usize>,
    /// The hour being chosen to sleep until (the same popup asks it), and the one chosen, for the game to take (`take_sleep`).
    sleep: Option<f32>,
    slept: Option<f32>,
    sleep_dragging: bool,
    /// The points a level gives to spend on health, mana or stamina, those spent on each, whether the panel for them is open, the button of it that is
    /// pressed, and its pictures (`levelup.rs`).
    points: u32,
    spent: [u32; 3],
    level_open: bool,
    level_press: Option<usize>,
    level_art: Option<levelup::Art>,
    /// The portrait of the target frame (offscreen picture), and which creature the frame shows.
    target_portrait: (usize, usize),
    target: Option<usize>,
    /// The game menu and the sound options.
    menu: menu::MenuState,
    keys: keybinds::KeyState,
}

/// The three screens of the quest window (`QuestFrameDetailPanel`, `QuestFrameProgressPanel`, `QuestFrameRewardPanel`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuestScreen {
    /// A quest is offered: accept or decline.
    Detail,
    /// A quest is taken: what is still needed.
    Progress,
    /// A quest is done: the reward.
    Reward,
}

/// What a button of the talk window stands for.
#[derive(Clone, Copy, Debug)]
enum Line {
    Available(usize),
    Active(usize),
    Option(usize),
}

const QUEST: &str = "QuestFrame";
const QUESTLOG: &str = "QuestLogFrame";
const POPUP: &str = "StaticPopup1";
/// The scroll frames of the windows: each shows its scroll bar (arrows and knob) when there is more to see.
const SCROLL_FRAMES: [&str; 6] = ["TalentFrameScrollFrame", "GossipGreetingScrollFrame", "QuestDetailScrollFrame", "QuestProgressScrollFrame", "QuestRewardScrollFrame", "QuestLogDetailScrollFrame"];
/// Quests listed at once, and goals shown for a quest, in the log.
const LOG_LINES: usize = 6;
const LOG_GOALS: usize = 10;
const QUEST_ITEMS: usize = 10;
const PROGRESS_ITEMS: usize = 6;
/// The text colour of a quest on the parchment (`GetMaterialTextColors`), and the yellow of the messages that are not errors.
const PARCHMENT: [f32; 4] = [0.18, 0.12, 0.06, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
const GREY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
const GREEN: [f32; 4] = [0.1, 1.0, 0.1, 1.0];
const GOLD: [f32; 4] = [1.0, 0.82, 0.0, 1.0];
/// How many messages stand at the top of the screen at most.
const MAX_MESSAGES: usize = 5;

/// The names of the schools of magic by the number of the original's item tables (0 physical), the classes and races of the masks of its items, and the skills
/// that items ask for.
const SCHOOLS: [&str; 7] = ["Physical", "Holy", "Fire", "Nature", "Frost", "Shadow", "Arcane"];
const CLASSES: [(u32, &str); 9] = [(1, "Warrior"), (2, "Paladin"), (4, "Hunter"), (8, "Rogue"), (16, "Priest"), (64, "Shaman"), (128, "Mage"), (256, "Warlock"), (1024, "Druid")];
const RACES: [(u32, &str); 8] = [(1, "Human"), (2, "Orc"), (4, "Dwarf"), (8, "Night Elf"), (16, "Undead"), (32, "Tauren"), (64, "Gnome"), (128, "Troll")];

fn skill_name(id: u32) -> String {
    match id {
        40 => "Poisons".into(),
        762 => "Riding".into(),
        129 => "First Aid".into(),
        164 => "Blacksmithing".into(),
        165 => "Leatherworking".into(),
        171 => "Alchemy".into(),
        182 => "Herbalism".into(),
        185 => "Cooking".into(),
        186 => "Mining".into(),
        197 => "Tailoring".into(),
        202 => "Engineering".into(),
        333 => "Enchanting".into(),
        356 => "Fishing".into(),
        393 => "Skinning".into(),
        633 => "Lockpicking".into(),
        other => format!("Skill {other}"),
    }
}

const MERCHANT: &str = "MerchantFrame";
const MERCHANT_ITEMS: usize = 10;

const GOSSIP: &str = "GossipFrame";
const GOSSIP_BUTTONS: usize = 32;

const LOOT: &str = "LootFrame";
const LOOT_BUTTONS: usize = 4;
/// Seconds an error message stays, and how long of that it fades.
const ERROR_SECONDS: f32 = 5.0;
const ERROR_FADE: f32 = 2.0;
const RED: [f32; 4] = [1.0, 0.1, 0.1, 1.0];

/// The button of the backpack for item slot `slot` (0-based): the buttons are numbered from the bottom right.
fn bag_button(slot: usize) -> String {
    format!("{BACKPACK}Item{}", BACKPACK_SIZE - slot)
}

/// The text of the original with the player filled in, for the one named `name` of race `race`.
fn speak_to(name: &str, race: crate::character::Race, text: &str) -> String {
    let race = race.name();
    text.replace("$N", name).replace("$n", name).replace("$R", race).replace("$r", &race.to_lowercase()).replace("$C", "Paladin").replace("$c", "paladin")
}

impl Panels {
    pub fn new(mount: Arc<Mount>, renderer: &mut Renderer, gpu: &Gpu) -> Result<Panels> {
        let registry = Rc::new(Registry::load(&mount)?);
        let mut tree = Tree::new(registry);
        tree.instantiate(CHARACTER)?;
        tree.instantiate(BACKPACK)?;
        tree.instantiate(LOOT)?;
        // Nothing here needs a second page of loot.
        for part in ["LootFramePrev", "LootFrameNext"] {
            tree.set_region_hidden(part, true);
        }
        tree.show("LootFrameUpButton", false);
        tree.show("LootFrameDownButton", false);
        tree.instantiate(GOSSIP)?;
        // The greeting is short enough to need no scroll bar; the portrait is the book until creatures get faces.
        tree.show("GossipGreetingScrollFrameScrollBar", false);
        tree.set_texture("GossipFramePortrait", Some(r"Interface\QuestFrame\UI-QuestLog-BookIcon"));
        for i in 1..=GOSSIP_BUTTONS {
            tree.show(&format!("GossipTitleButton{i}"), false);
        }
        tree.instantiate(QUEST)?;
        // The three screens show one at a time; no scroll bars (the texts fit), and the book for a portrait.
        for panel in ["QuestFrameDetailPanel", "QuestFrameProgressPanel", "QuestFrameRewardPanel", "QuestFrameGreetingPanel"] {
            tree.show(panel, false);
        }
        for bar in ["QuestDetailScrollFrameScrollBar", "QuestProgressScrollFrameScrollBar", "QuestRewardScrollFrameScrollBar", "QuestGreetingScrollFrameScrollBar"] {
            tree.show(bar, false);
        }
        tree.set_texture("QuestFramePortrait", Some(r"Interface\QuestFrame\UI-QuestLog-BookIcon"));
        tree.instantiate(POPUP)?;
        sleep::setup(&mut tree)?;
        // A question with two answers: no close button, no edit boxes, no money, no alert picture.
        for part in ["StaticPopup1CloseButton", "StaticPopup1EditBox", "StaticPopup1WideEditBox", "StaticPopup1MoneyFrame"] {
            tree.show(part, false);
        }
        tree.set_region_hidden("StaticPopup1AlertIcon", true);
        tree.set_size("StaticPopup1Button1", Some(120.0), None);
        tree.set_size("StaticPopup1Button2", Some(120.0), None);
        tree.set_point("StaticPopup1Button1", Point::TopRight, Some("StaticPopup1Text"), Some(Point::Bottom), -6.0, -8.0);
        tree.set_point("StaticPopup1Button2", Point::Left, Some("StaticPopup1Button1"), Some(Point::Right), 13.0, 0.0);
        tree.instantiate(QUESTLOG)?;
        // One list (no zone headers, no sharing, no tracking checkbox) and no scroll bars: six quests fit.
        for part in ["QuestLogExpandButtonFrame", "QuestLogTrack", "QuestFramePushQuestButton", "QuestLogListScrollFrameScrollBar", "QuestLogDetailScrollFrameScrollBar", "QuestLogListScrollFrame", "EmptyQuestLogFrame", "QuestLogHighlightFrame"] {
            tree.show(part, false);
        }
        tree.instantiate(MERCHANT)?;
        // One page of wares, no repairs, and no buyback list: the last thing sold stays in its slot at the bottom.
        for i in 11..=12 {
            tree.show(&format!("MerchantItem{i}"), false);
        }
        for button in ["MerchantRepairAllButton", "MerchantRepairItemButton", "MerchantPrevPageButton", "MerchantNextPageButton", "MerchantFrameTab2"] {
            tree.show(button, false);
        }
        for region in ["MerchantRepairText", "MerchantPageText", "BuybackFrameTopLeft", "BuybackFrameTopRight", "BuybackFrameBotLeft", "BuybackFrameBotRight", "MerchantFrameTab1Left", "MerchantFrameTab1Middle", "MerchantFrameTab1Right"] {
            tree.set_region_hidden(region, true);
        }
        tree.set_texture("MerchantFramePortrait", Some(r"Interface\MerchantFrame\UI-BuyBack-Icon"));
        // The buffs: what works on the character (an elixir, food being eaten) as the icons at the top right, shown while there are any.
        tree.instantiate("BuffFrame")?;
        tree.show("BuffFrame", false);
        // The durations under the icons are an option of the original that is off unless it is turned on (`SHOW_BUFF_DURATIONS`).
        // BuffFrame.lua puts each one under its icon (`BuffFrame_OnLoad`), which XML alone does not.
        for i in 0..24 {
            tree.set_region_hidden(&format!("BuffButton{i}Duration"), true);
            tree.set_region_font(&format!("BuffButton{i}Duration"), "GameFontNormalSmall");
            tree.set_region_point(&format!("BuffButton{i}Duration"), Point::Top, Some(&format!("BuffButton{i}")), Some(Point::Bottom), 0.0, 0.0);
        }
        // The target frame: hidden until something is targeted, its bars under the frame's art and a picture for the portrait.
        tree.instantiate("TargetFrame")?;
        tree.show("TargetFrame", false);
        for bar in ["TargetFrameHealthBar", "TargetFrameManaBar"] {
            tree.set_level(bar, -1);
        }
        // Creatures here have no mana to show.
        tree.show("TargetFrameManaBar", false);
        // No auras, and no target of the target.
        for i in 1..=16 {
            tree.show(&format!("TargetFrameDebuff{i}"), false);
        }
        for i in 1..=5 {
            tree.show(&format!("TargetFrameBuff{i}"), false);
        }
        tree.show("TargetofTargetFrame", false);
        for part in ["TargetFrameDropDown", "TargetLeaderIcon", "TargetPVPIcon", "TargetRaidTargetIcon"] {
            tree.show(part, false);
            tree.set_region_hidden(part, true);
        }
        let target_portrait = renderer.add_offscreen(gpu, PORTRAIT_SIZE);
        tree.set_region_texture("TargetPortrait", Some(target_portrait.1));
        let inv = Inventory::new();
        let (doll, portrait) = (renderer.add_offscreen(gpu, DOLL_SIZE), renderer.add_offscreen(gpu, PORTRAIT_SIZE));
        tree.set_model_texture("CharacterModelFrame", Some(doll.1));
        tree.set_region_texture("CharacterFramePortrait", Some(portrait.1));

        // Only the character tab: the others would show frames this game has nothing to put in yet.
        for tab in 2..=5 {
            tree.show(&format!("CharacterFrameTab{tab}"), false);
        }
        // The pet frame is only shown with a pet; the ammunition slot hides when the ranged slot is a relic slot.
        tree.show("PetPaperDollFrame", false);
        magic::setup(&mut tree)?;
        talents::setup(&mut tree)?;
        trade::setup(&mut tree)?;
        trainer::setup(&mut tree)?;
        tree.set_region_texture("TradeSkillFramePortrait", Some(portrait.1));
        tree.set_region_texture("TalentFramePortrait", Some(portrait.1));
        menu::setup(&mut tree)?;

        // The backpack, as `ContainerFrame_GenerateFrame` lays out bag 0: its own picture, 16 slots in 4 columns
        // numbered from the bottom right, scaled down and standing above the bottom right corner.
        tree.set_texture("ContainerFrame1BackgroundTop", Some(r"Interface\ContainerFrame\UI-BackpackBackground"));
        tree.set_region_size("ContainerFrame1BackgroundTop", None, Some(256.0));
        tree.set_region_coords("ContainerFrame1BackgroundTop", [0.0, 1.0, 0.0, 1.0]);
        for i in 1..=2 {
            tree.set_region_hidden(&format!("ContainerFrame1BackgroundMiddle{i}"), true);
        }
        tree.set_region_hidden("ContainerFrame1BackgroundBottom", true);
        tree.set_size(BACKPACK, Some(192.0), Some(240.0));
        tree.set_scale(BACKPACK, CONTAINER_SCALE);
        tree.set_point(BACKPACK, Point::BottomRight, Some("UIParent"), Some(Point::BottomRight), 0.0, CONTAINER_OFFSET_Y);
        tree.set_text("ContainerFrame1Name", BACKPACK_NAME);
        tree.set_texture("ContainerFrame1Portrait", Some(BACKPACK_PORTRAIT));
        tree.show("ContainerFrame1MoneyFrame", true);
        for j in 1..=BACKPACK_SIZE {
            let name = format!("{BACKPACK}Item{j}");
            if j == 1 {
                tree.set_point(&name, Point::BottomRight, Some(BACKPACK), Some(Point::BottomRight), -12.0, 30.0);
            } else if (j - 1) % CONTAINER_COLUMNS == 0 {
                tree.set_point(&name, Point::BottomRight, Some(&format!("{BACKPACK}Item{}", j - CONTAINER_COLUMNS)), Some(Point::TopRight), 0.0, 4.0);
            } else {
                tree.set_point(&name, Point::BottomRight, Some(&format!("{BACKPACK}Item{}", j - 1)), Some(Point::BottomLeft), -5.0, 0.0);
            }
            tree.show(&name, true);
        }

        let mut panels = Panels { name: "Adventurer".into(), race: Default::default(), tree, mount, stats: Stats::of(&inv.equipment, 1, &[0; 7], &[]), progress: Progress::new(), found: vec![false; content::get().pois.len()], map_open: false, map: Default::default(), explored: Vec::new(), travel: None, skills: crate::skills::Skills::new(), recipes: Default::default(), known: content::get().start.spells.clone(), learned: Vec::new(), forgotten: Vec::new(), cooldowns: vec![0.0; content::get().spells.len()], book_page: 0, cast_bar: Default::default(), breath_shown: false, spell_click: None, drag: None, skill_list: Default::default(), talents: Default::default(), trade: Default::default(), trainer: Default::default(), craft_request: None, talent_tab: 0, inv, bag_scroll: 0, doll, portrait, doll_turn: 0.0, cursor: None, equipment_changed: false, stats_dirty: false, loot: None, closed_loot: None, events: Vec::new(), messages: Vec::new(), gossip: None, merchant: None, merchant_page: 0, consumed: Default::default(), afflicted: Vec::new(), calm: true, restored: (0.0, 0.0), steed: None, mounted: false, mount_cast: None, lure: (None, false), buyback: None, quests: QuestLog::default(), quest_targets: Vec::new(), gossip_lines: Vec::new(), questing: None, seen: Default::default(), ready: Default::default(), log_selected: 0, reward_choice: None, level_ups: 0, popup: None, sleep: None, slept: None, sleep_dragging: false, points: 0, spent: [0; 3], level_open: false, level_press: None, level_art: None, target_portrait, target: None, menu: Default::default(), keys: Default::default() };
        panels.refresh();
        Ok(panels)
    }

    pub fn character_open(&self) -> bool {
        self.tree.is_name_shown(CHARACTER)
    }

    pub fn backpack_open(&self) -> bool {
        self.tree.is_name_shown(BACKPACK)
    }

    /// The corpse whose loot window is open.
    pub fn looting(&self) -> Option<usize> {
        self.loot.as_ref().map(|(corpse, _)| *corpse)
    }

    pub fn loot_open(&self) -> bool {
        self.loot.is_some()
    }

    pub fn gossip_open(&self) -> bool {
        self.gossip.is_some()
    }

    pub fn merchant_open(&self) -> bool {
        self.merchant.is_some()
    }

    /// The creature being talked or traded with.
    pub fn talking_to(&self) -> Option<usize> {
        self.gossip.as_ref().map(|(creature, ..)| *creature).or(self.merchant.map(|(creature, _)| creature)).or(self.trainer_creature()).or(self.questing.map(|(creature, ..)| creature))
    }

    pub fn quest_open(&self) -> bool {
        self.questing.is_some()
    }

    pub fn quest_log_open(&self) -> bool {
        self.tree.is_name_shown(QUESTLOG)
    }

    pub fn popup_open(&self) -> bool {
        self.popup.is_some() || self.sleep.is_some()
    }

    pub fn any_open(&self) -> bool {
        self.level_open || self.windows_open()
    }

    /// Whether a window of the original's interface is open (the level-up panel is not one: it gives way to them).
    fn windows_open(&self) -> bool {
        self.popup_open() || self.character_open() || self.backpack_open() || self.loot_open() || self.gossip_open() || self.merchant_open() || self.quest_open() || self.quest_log_open() || self.spellbook_open() || self.talents_open() || self.trade_open() || self.trainer_open() || self.map_open || self.menu_open() || self.options_open()
    }

    // ---- the question

    /// Ask whether the selected quest of the log is to be abandoned (`StaticPopupDialogs["ABANDON_QUEST"]`).
    fn ask_abandon(&mut self) {
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

    fn hide_log(&mut self) {
        self.tree.show(QUESTLOG, false);
        self.tree.show(POPUP, false);
        self.popup = None;
        self.sleep = None;
    }

    /// `QuestLog_Update` and `QuestLog_UpdateQuestDetails`: the list of quests, and the selected one in detail.
    fn fill_log(&mut self) {
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

    fn fill_log_details(&mut self) {
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
    fn counted_text(&self, key: &str, name: &str, quest: usize, goal: usize, need: u32) -> String {
        self.counted(key, name, self.quests.have(quest, goal, &self.inv), need)
    }

    fn select_log(&mut self, line: usize) {
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
    fn abandon_asked(&mut self) {
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

    // ---- the merchant

    /// Open the merchant window for creature `creature` of kind `def` (with the backpack, as the original does).
    pub fn open_merchant(&mut self, creature: usize, def: &'static CreatureDef) {
        self.gossip = None;
        self.tree.show(GOSSIP, false);
        self.merchant = Some((creature, def));
        self.merchant_page = 0;
        self.tree.show(MERCHANT, true);
        self.tree.show(BACKPACK, true);
        self.events.push(Sfx::Named("igBackPackOpen"));
        self.fill_merchant();
    }

    pub fn close_merchant(&mut self) {
        if self.merchant.take().is_some() {
            self.tree.show(BACKPACK, false);
            self.events.push(Sfx::Named("igBackPackClose"));
        }
        self.tree.show(MERCHANT, false);
        self.after_close();
    }

    /// `MerchantFrame_UpdateMerchantInfo`: the wares with their prices, and the last thing sold in the buyback slot.
    fn fill_merchant(&mut self) {
        let Some((_, def)) = self.merchant else { return };
        self.tree.set_text("MerchantNameText", &def.name);
        // More than ten wares make pages, with the buttons and the page number of the original.
        let pages = def.vendor.len().div_ceil(MERCHANT_ITEMS).max(1);
        self.merchant_page = self.merchant_page.min(pages - 1);
        for button in ["MerchantPrevPageButton", "MerchantNextPageButton"] {
            self.tree.show(button, pages > 1);
        }
        let page_text = self.string("PAGE_NUMBER").replacen("%d", &(self.merchant_page + 1).to_string(), 1);
        self.tree.set_text("MerchantPageText", &page_text);
        self.tree.set_region_hidden("MerchantPageText", pages <= 1);
        for i in 1..=MERCHANT_ITEMS {
            let (frame, button) = (format!("MerchantItem{i}"), format!("MerchantItem{i}ItemButton"));
            match def.vendor.get(self.merchant_page * MERCHANT_ITEMS + i - 1) {
                Some(&item) => {
                    let it = crate::items::get(item);
                    self.tree.set_text(&format!("{frame}Name"), it.name);
                    self.tree.set_texture(&format!("{button}IconTexture"), Some(it.icon));
                    // What a merchant sells at a time (arrows by the hundred) is the number on the button.
                    self.tree.set_text(&format!("{button}Count"), &it.buy_count.to_string());
                    self.tree.set_region_hidden(&format!("{button}Count"), it.buy_count <= 1);
                    self.tree.set_money(&format!("{frame}MoneyFrame"), it.buy, true, false);
                    self.tree.show(&format!("{frame}MoneyFrame"), true);
                    self.tree.show(&button, true);
                }
                None => {
                    self.tree.set_text(&format!("{frame}Name"), "");
                    self.tree.show(&format!("{frame}MoneyFrame"), false);
                    self.tree.show(&button, false);
                }
            }
        }
        let back = "MerchantBuyBackItem";
        match self.buyback {
            Some((stack, price)) => {
                let it = crate::items::get(stack.item);
                self.tree.set_text(&format!("{back}Name"), it.name);
                self.tree.set_texture(&format!("{back}ItemButtonIconTexture"), Some(it.icon));
                self.tree.set_text(&format!("{back}ItemButtonCount"), &stack.count.to_string());
                self.tree.set_region_hidden(&format!("{back}ItemButtonCount"), stack.count <= 1);
                self.tree.set_money(&format!("{back}MoneyFrame"), price, true, false);
                self.tree.show(&format!("{back}MoneyFrame"), true);
            }
            None => {
                self.tree.set_text(&format!("{back}Name"), "");
                self.tree.set_texture(&format!("{back}ItemButtonIconTexture"), None);
                self.tree.set_region_hidden(&format!("{back}ItemButtonCount"), true);
                self.tree.show(&format!("{back}MoneyFrame"), false);
            }
        }
    }

    /// The error for a purchase the purse or the pack cannot take.
    fn refuse(&mut self, why: crate::items::Refusal) {
        let key = match why {
            crate::items::Refusal::TooPoor => "ERR_NOT_ENOUGH_MONEY",
            crate::items::Refusal::PackFull => "ERR_INV_FULL",
        };
        self.error(&self.string(key));
    }

    /// Turn to a page of the wares (for previews).
    pub fn merchant_page_to(&mut self, page: usize) {
        self.merchant_page = page;
        self.fill_merchant();
    }

    /// Click on ware `n` (counted from 1): buy one.
    fn buy_ware(&mut self, n: usize) {
        let Some(&item) = self.merchant.and_then(|(_, def)| def.vendor.get(self.merchant_page * MERCHANT_ITEMS + n - 1)) else { return };
        match self.inv.buy(item) {
            Ok(()) => {
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
                self.events.push(Sfx::PickUp(crate::items::get(item).handling));
            }
            Err(why) => self.refuse(why),
        }
    }

    /// Click on the buyback slot: get the last thing sold back for what it fetched.
    fn buy_back(&mut self) {
        let Some((stack, price)) = self.buyback else { return };
        match self.inv.pay(stack, price) {
            Ok(()) => {
                self.buyback = None;
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
                self.events.push(Sfx::PickUp(crate::items::get(stack.item).handling));
            }
            Err(why) => self.refuse(why),
        }
    }

    /// Right-click on a backpack item with a merchant open: sell it (the whole stack).
    fn sell_item(&mut self, slot: usize) {
        match self.inv.sell(slot) {
            Some((stack, price)) => {
                self.buyback = Some((stack, price));
                self.events.push(Sfx::PutDown(crate::items::get(stack.item).handling));
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
            }
            None if self.inv.bag[slot].is_some() => self.error(&self.string("ERR_VENDOR_NOT_INTERESTED")),
            None => {}
        }
    }

    // ---- gossip

    /// What a creature or quest says, with the player filled in: `$N` the name, `$R` the race, `$C` the class (`$n`, `$r`, `$c` as the original writes them, race and class in lower case).
    fn speak(&self, text: &str) -> String {
        speak_to(&self.name, self.race, text)
    }

    /// Open the talk window for creature `creature` of kind `def`. The character sheet and the loot window close.
    pub fn open_gossip(&mut self, creature: usize, def: &'static CreatureDef) {
        self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
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

    fn hide_gossip(&mut self) {
        self.gossip = None;
        self.tree.show(GOSSIP, false);
    }

    /// `GossipFrameUpdate`: the name, the text (the greeting, or the last answer) and a button for each line to choose:
    /// the quests the creature gives, the quests it takes back, then what else it offers.
    fn fill_gossip(&mut self) {
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
    fn choose_gossip(&mut self, n: usize) {
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
    fn learn_skill(&mut self, skill: crate::skills::Skill, rank: u32, needs: u32, level: u32, cost: u32) -> String {
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

    fn hide_quest(&mut self) {
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
    fn set_button(&mut self, button: &str, enabled: bool) {
        self.tree.set_enabled(button, enabled);
        self.tree.set_text_color(&format!("{button}Text"), if enabled { GOLD } else { GREY });
    }

    /// The reward part of the detail, reward and log screens (`QuestFrameItems_Update`): the title, the items to choose from
    /// ("Choose your reward:"), the line "You will receive:" with the money next to it, and the items, two to a row. The
    /// buttons number the items to choose from first, then those everyone gets.
    fn fill_rewards(&mut self, prefix: &str, quest: usize) {
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
    fn show_stack(&mut self, button: &str, stack: Stack) {
        let item = crate::items::get(stack.item);
        self.tree.set_texture(&format!("{button}IconTexture"), Some(item.icon));
        self.tree.set_text(&format!("{button}Name"), item.name);
        self.tree.set_text(&format!("{button}Count"), &stack.count.to_string());
        self.tree.set_region_hidden(&format!("{button}Count"), stack.count <= 1);
        self.tree.show(button, true);
    }

    /// `QuestFrameDetailPanel_OnShow` and its two sisters: the texts of the quest and the buttons of the screen.
    fn fill_quest(&mut self) {
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
    fn quest_click(&mut self, button: &str) -> bool {
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
    fn refresh_quest_marks(&mut self, quest: usize) {
        for (g, goal) in content::quest(quest).goals.iter().enumerate() {
            if matches!(goal, Goal::Collect { .. }) {
                self.seen.insert((quest, g), self.quests.have(quest, g, &self.inv));
            }
        }
        if self.quests.complete(quest, &self.inv) {
            self.ready.insert(quest);
        }
    }

    /// After a saved game was loaded into `quests`: the quests start counting from what is carried, without announcing it.
    /// Put a saved character into the windows' state: what it carries, its level, quests, skills, spells and discoveries. What the saved game no
    /// longer has under its id is dropped (see `Save::apply`).
    /// The character's name and race, which the character sheet, the quest texts and the tooltips of items go by.
    pub fn set_identity(&mut self, name: &str, race: crate::character::Race) {
        (self.name, self.race) = (name.to_string(), race);
        content::set_horde(race.horde());
    }

    pub fn restore(&mut self, saved: &crate::save::Save) {
        for note in saved.apply(&mut self.inv, &mut self.progress, &mut self.quests) {
            log::warn!("saved game: {note}");
        }
        self.spent = saved.spent;
        self.talents = crate::talents::Ranks::from_saved(&saved.talents);
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
    fn counted(&self, key: &str, name: &str, have: u32, need: u32) -> String {
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
    pub fn toggle_character(&mut self) {
        let on = !self.character_open();
        self.tree.show(CHARACTER, on);
        self.show_skills_tab(self.skill_list.shown);
        if on {
            self.tree.show(SPELLBOOK, false);
            self.tree.show(TALENTS, false);
        }
        // The character sheet and the loot window are both left panels: only one of them is open.
        if on {
            self.tree.show(trade::TRADE, false);
            self.close_loot();
            self.close_gossip();
            self.close_merchant();
            self.close_quest();
            self.hide_log();
        }
        self.events.push(Sfx::Named(if on { "igCharacterInfoOpen" } else { "igCharacterInfoClose" }));
        self.after_close();
    }

    pub fn toggle_backpack(&mut self) {
        let on = !self.backpack_open();
        self.tree.show(BACKPACK, on);
        self.events.push(Sfx::Named(if on { "igBackPackOpen" } else { "igBackPackClose" }));
        self.after_close();
    }

    pub fn close_all(&mut self) {
        if self.keys_open() {
            self.keys_done(false);
        }
        self.map_open = false;
        self.level_open = false;
        self.tree.show("GameMenuFrame", false);
        self.tree.show("SoundOptionsFrame", false);
        self.tree.show("OptionsFrame", false);
        self.tree.show("UIOptionsFrame", false);
        self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
        self.tree.show(trade::TRADE, false);
        self.tree.show(BACKPACK, false);
        self.close_loot();
        self.close_gossip();
        self.close_trainer();
        self.close_merchant();
        self.close_quest();
        self.hide_log();
        self.after_close();
    }

    /// The sounds the interface made since the last call.
    pub fn take_events(&mut self) -> Vec<Sfx> {
        std::mem::take(&mut self.events)
    }

    /// How many lockpicks the character carries.
    pub fn picks(&self) -> u32 {
        self.inv.count(crate::items::find("lockpick"))
    }

    /// A pick breaks: one is gone from the backpack.
    pub fn use_pick(&mut self) {
        self.inv.remove(crate::items::find("lockpick"), 1);
        self.refresh();
    }

    /// Make a sound of the interface (a `SoundEntries` name).
    /// A sound to play with the next `take_events` (the spells' own, by entry).
    pub fn emit(&mut self, sfx: Sfx) {
        self.events.push(sfx);
    }

    pub fn sound(&mut self, name: &'static str) {
        self.events.push(Sfx::Named(name));
    }

    /// Queue the words of lockpicking over the lock (`lock` itself is drawn by the lock).
    pub fn draw_lockpick(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), lock: &crate::lockpick::Lockpick, art: Option<&crate::lockpick::LockArt>, moment: f32) {
        if let Some(art) = art {
            lock.draw(ui, art, screen, moment);
        }
        let mount = self.mount.clone();
        let s = screen.1 as f32 / ui_height();
        let x = screen.0 as f32 / 2.0;
        // The icon of the Pick Lock spell beside the number of picks left.
        let icon = 32.0 * s;
        self.tree.draw_icon(ui, gpu, &mount, crate::lockpick::PICK_ICON, [x - 70.0 * s - icon, 120.0 * s, icon, icon]);
        let lines = [
            ("GameFontNormalLarge", format!("{} lock", lock.level.name()), 60.0, GOLD),
            ("GameFontHighlight", format!("Lockpicks: {}", self.picks()), 128.0, WHITE),
            ("GameFontHighlightSmall", "Move the mouse to turn the pick. Hold A, D, Space or the left button to turn the lock.".to_string(), 88.0, GREY),
            ("GameFontHighlightSmall", "Too far from the sweet spot and the pick strains, and breaks. Esc to leave.".to_string(), 104.0, GREY),
        ];
        for (font, text, y, color) in lines {
            self.tree.draw_text_aligned(ui, gpu, &mount, screen, font, &text, (x, y * s), 0.5, color);
        }
    }

    /// The icon of sneaking at the top right, where the original shows the buffs of the character (`BuffFrame`), with its name under it.
    pub fn draw_sneak(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        let mount = self.mount.clone();
        let s = screen.1 as f32 / ui_height();
        let (x, y, size) = (screen.0 as f32 - 205.0 * s - 32.0 * s, 13.0 * s, 32.0 * s);
        self.tree.draw_icon(ui, gpu, &mount, r"Interface\Icons\Ability_Stealth", [x, y, size, size]);
        self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormalSmall", "Sneaking", (x + size / 2.0, y + size + 2.0 * s), 0.5, WHITE);
    }

    /// Discover the places the character at `feet` has come close to: each says so, with its sound. Returns whether there was one.
    pub fn discover(&mut self, feet: Vec3) -> bool {
        let new = crate::compass::discoveries(feet, &self.found);
        for &i in &new {
            self.found[i] = true;
            self.notice(&format!("Discovered: {}", content::get().pois[i].name));
            self.events.push(Sfx::Named("TaxiNodeDiscovered"));
        }
        !new.is_empty()
    }

    /// The compass at the top middle of the screen, for a character at `pos` (render space) facing `yaw`: a band with ticks every
    /// 15 degrees (the points of the compass named), fading towards its ends, and the places near, the farther the fainter.
    pub fn draw_compass(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), pos: Vec3, yaw: f32, nodes: &[(f32, f32, crate::skills::Skill, &str)]) {
        use crate::compass::{marks, relative, HALF_VIEW};
        let mount = self.mount.clone();
        let s = screen.1 as f32 / ui_height();
        let (cx, top, half) = (screen.0 as f32 / 2.0, 14.0 * s, 230.0 * s);
        let fade = |x: f32| (1.0 - x * x * x * x).clamp(0.0, 1.0);
        let quad = |ui: &mut Ui, rect: [f32; 4], color: [f32; 4]| ui.quad(Quad { texture: crate::ui::WHITE, rect, uv: FULL_UV, color, additive: false });
        // The band's line, in short pieces so that it fades out.
        for i in 0..40 {
            let x = (i as f32 + 0.5) / 20.0 - 1.0;
            quad(ui, [cx + (x - 0.025) * half, top + 10.0 * s, 0.05 * half, 1.5 * s], [1.0, 1.0, 1.0, 0.55 * fade(x)]);
        }
        for step in 0..24 {
            let x = relative(step as f32 * 15f32.to_radians(), yaw) / HALF_VIEW;
            if x.abs() > 1.0 {
                continue;
            }
            let long = step % 6 == 0;
            let h = if long { 8.0 } else if step % 2 == 0 { 6.0 } else { 3.5 } * s;
            quad(ui, [cx + x * half - 0.5 * s, top + 10.0 * s - h, 1.5 * s, h], [1.0, 1.0, 1.0, 0.8 * fade(x)]);
            if long {
                let name = ["N", "E", "S", "W"][step / 6];
                self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlight", name, (cx + x * half, top - 8.0 * s), 0.5, [1.0, 1.0, 1.0, fade(x)]);
            }
        }
        // The notch at the middle, where the player faces.
        ui.set_turn(Some((45f32.to_radians(), cx, top + 16.0 * s)));
        quad(ui, [cx - 2.5 * s, top + 13.5 * s, 5.0 * s, 5.0 * s], [1.0, 0.82, 0.0, 0.9]);
        ui.set_turn(None);
        for m in marks(pos, yaw) {
            let (x, fading) = (cx + m.x * half, fade(m.x) * (1.0 - 0.6 * m.distance / crate::compass::RANGE));
            let at = (x, top + 24.0 * s);
            if self.found[m.poi] {
                if let Some(texture) = self.tree.texture(ui, gpu, &mount, crate::compass::POI_ICONS) {
                    let size = 16.0 * s;
                    ui.quad(Quad { texture, rect: [at.0 - size / 2.0, at.1 - size / 2.0, size, size], uv: crate::compass::poi_uv(content::get().pois[m.poi].icon), color: [1.0, 1.0, 1.0, fading], additive: false });
                }
                // The name shows only for the place the player is facing.
                if m.x.abs() < 0.3 {
                    self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormalSmall", &content::get().pois[m.poi].name, (at.0, at.1 + 8.0 * s), 0.5, [1.0, 1.0, 1.0, fading]);
                }
            } else {
                self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormalLarge", "?", (at.0, at.1 - 9.0 * s), 0.5, [0.75, 0.75, 0.75, fading]);
            }
        }
        // The veins and herbs near: the icon of the profession that gathers each, small, the farther the fainter.
        for &(x, distance, skill, _) in nodes {
            let icon = if skill == crate::skills::Skill::Mining { r"Interface\Icons\Trade_Mining" } else { r"Interface\Icons\Spell_Nature_NatureTouchGrow" };
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, icon) {
                let size = 14.0 * s;
                let fading = fade(x) * (1.0 - 0.6 * distance / crate::compass::RANGE);
                ui.quad(Quad { texture, rect: [cx + x * half - size / 2.0, top + 22.0 * s - size / 2.0, size, size], uv: FULL_UV, color: [1.0, 1.0, 1.0, fading], additive: false });
            }
        }
        // The name shows only for the node the player is facing (the nearest of those in front), as a place's does.
        if let Some(&(x, distance, _, name)) = nodes.iter().filter(|n| n.0.abs() < 0.3).min_by(|a, b| a.1.total_cmp(&b.1)) {
            let fading = fade(x) * (1.0 - 0.6 * distance / crate::compass::RANGE);
            self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormalSmall", name, (cx + x * half, top + 36.0 * s), 0.5, [1.0, 1.0, 1.0, fading]);
        }
        // The arrow of the tracked quest (the guide arrow of the original's minimap): over the band when its place is in view,
        // turned to point along the band's end when it is not.
        let Some(quest) = self.quests.tracked else { return };
        let Some((x, out, distance)) = crate::compass::nearest(pos, yaw, &self.quest_targets.iter().map(|t| t.0).collect::<Vec<_>>()) else { return };
        let (ax, ay, size) = (cx + x * half, top + 36.0 * s, 34.0 * s);
        let turn = if !out { 0.0 } else if x < 0.0 { 3.0 * PI / 2.0 } else { PI / 2.0 };
        if let Some(texture) = self.tree.texture(ui, gpu, &mount, r"Interface\Minimap\ROTATING-MINIMAPGUIDEARROW") {
            ui.set_turn(Some((turn, ax, ay)));
            ui.quad(Quad { texture, rect: [ax - size / 2.0, ay - size / 2.0, size, size], uv: FULL_UV, color: WHITE, additive: false });
            ui.set_turn(None);
        }
        let label = format!("{} - {} yd", content::quest(quest).title, distance.round());
        self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormalSmall", &label, (cx, top + 58.0 * s), 0.5, WHITE);
    }

    /// Find where the tracked quest leads in the world as it is now: the creatures of its goal and the chests that hold its item.
    pub fn set_quest_targets(&mut self, creatures: Option<&crate::creature::Creatures>, chests: Option<&crate::chest::Chests>, pickups: Option<&crate::pickups::Pickups>) {
        let Some(lead) = self.quests.tracked.map(|q| self.quests.lead(q, &self.inv)) else {
            self.quest_targets.clear();
            return;
        };
        let creatures = creatures.map_or_else(Vec::new, |c| c.positions(&lead.kinds));
        let chests = lead.item.map_or_else(Vec::new, |item| chests.map_or_else(Vec::new, |c| c.holding(item)).into_iter().chain(pickups.map_or_else(Vec::new, |p| p.holding(item))).collect());
        let places = lead.places.iter().map(|&(north, west)| crate::coords::wow_to_render([north, west, 0.0]));
        self.quest_targets = creatures.into_iter().map(|p| (p, lead.icon)).chain(places.map(|p| (p, lead.icon))).chain(chests.into_iter().map(|p| (p, crate::quests::Icon::Chest))).collect();
    }

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
    fn enchant_effects(&self) -> Vec<crate::items::UseEffect> {
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

    /// Give a spell back (a talent was unlearned): it is no longer known, and the action bar loses it.
    pub fn forget(&mut self, spell: usize) {
        if let Some(at) = self.known.iter().position(|&s| s == spell) {
            self.known.remove(at);
            self.forgotten.push(spell);
        }
    }

    /// The spells given back since the last call.
    pub fn take_forgotten(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.forgotten)
    }

    /// The spells learned since the last call.
    pub fn take_learned(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.learned)
    }

    pub fn tick_cooldowns(&mut self, dt: f32) {
        self.cooldowns.iter_mut().for_each(|c| *c = (*c - dt).max(0.0));
    }

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

    /// Whether the mouse carries an item (which is drawn instead of the pointer).
    pub fn carrying(&self) -> bool {
        self.inv.cursor.is_some()
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
    fn bag_index(&self, j: usize) -> usize {
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
    fn bag_entry_of(&self, name: &str) -> Option<crate::actionbar::Action> {
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
    fn use_item(&mut self, slot: usize) {
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
    fn learn_recipe_item(&mut self, item: usize, spells: &[u32]) {
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

    /// The health and mana that using items gave since the last call.
    pub fn take_restores(&mut self) -> (f32, f32) {
        std::mem::take(&mut self.restored)
    }

    /// Say why an item was not put on (the original's error messages).
    fn equip_error(&mut self, e: crate::items::EquipError) {
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

    fn message(&mut self, text: &str, color: [f32; 4]) {
        self.messages.push((text.to_string(), color, 0.0));
        if self.messages.len() > MAX_MESSAGES {
            self.messages.remove(0);
        }
    }

    // ---- loot

    /// Open the loot window for corpse `corpse` holding `loot`. The character sheet closes.
    pub fn open_loot(&mut self, corpse: usize, loot: Loot) {
        self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
        self.loot = Some((corpse, loot));
        self.tree.show(LOOT, true);
        self.fill_loot();
    }

    /// Close the loot window; what is left on the corpse is handed back (see `take_closed_loot`).
    pub fn close_loot(&mut self) {
        if let Some(open) = self.loot.take() {
            self.closed_loot = Some(open);
        }
        self.tree.show(LOOT, false);
        self.after_close();
    }

    /// The corpse a loot window closed on, and what is still on it.
    pub fn take_closed_loot(&mut self) -> Option<(usize, Loot)> {
        self.closed_loot.take()
    }

    /// The slots of the loot window in order: the money (`None`), then the items.
    fn loot_slots(loot: &Loot) -> Vec<Option<usize>> {
        (loot.copper > 0).then_some(None).into_iter().chain((0..loot.items.len()).map(Some)).collect()
    }

    /// `LootFrame_Update`: the buttons for what is on the corpse, each with its picture, name and count.
    fn fill_loot(&mut self) {
        let Some((_, loot)) = &self.loot else { return };
        let slots = Self::loot_slots(loot);
        for i in 1..=LOOT_BUTTONS {
            let button = format!("LootButton{i}");
            let Some(&slot) = slots.get(i - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            let (icon, name, count, color) = match slot {
                None => (coin_icon(loot.copper), self.coin_text(loot.copper), 0, [1.0; 4]),
                Some(k) => {
                    let (stack, item) = (loot.items[k], crate::items::get(loot.items[k].item));
                    let q = QUALITY_COLORS[item.quality];
                    (item.icon, item.name.to_string(), stack.count, [q[0], q[1], q[2], 1.0])
                }
            };
            self.tree.set_texture(&format!("{button}IconTexture"), Some(icon));
            self.tree.set_text(&format!("{button}Text"), &name);
            self.tree.set_text_color(&format!("{button}Text"), color);
            self.tree.set_text(&format!("{button}Count"), &count.to_string());
            self.tree.set_region_hidden(&format!("{button}Count"), count <= 1);
            self.tree.show(&button, true);
        }
    }

    /// "5 Copper", or one denomination to a line: "1 Silver\n20 Copper".
    fn coin_text(&self, copper: u32) -> String {
        [(copper / 10000, "GOLD"), (copper / 100 % 100, "SILVER"), (copper % 100, "COPPER")]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, key)| format!("{n} {}", self.string(key)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Click a loot button: the money goes into the purse, an item into the backpack (if it fits). An emptied
    /// corpse closes the window.
    fn take_loot(&mut self, button: usize) {
        let Some((_, loot)) = &mut self.loot else { return };
        let Some(&slot) = Self::loot_slots(loot).get(button - 1) else { return };
        match slot {
            None => {
                self.inv.money += std::mem::take(&mut loot.copper);
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
            }
            Some(k) => {
                let stack = loot.items[k];
                if self.inv.add(stack) {
                    loot.items.remove(k);
                    self.events.push(Sfx::PickUp(crate::items::get(stack.item).handling));
                } else {
                    self.error(&self.string("INVENTORY_FULL"));
                    return;
                }
            }
        }
        if loot.is_empty() {
            self.tree.show(trade::TRADE, false);
            self.close_loot();
        } else {
            self.fill_loot();
        }
    }

    /// Take everything from the open loot window; what does not fit in the pack stays on the corpse.
    pub fn loot_all(&mut self) {
        let left = |p: &Self| p.loot.as_ref().map(|(_, l)| Self::loot_slots(l).len());
        let mut skip = 0;
        while let Some(n) = left(self).filter(|&n| skip < n) {
            self.take_loot(skip + 1);
            if left(self) == Some(n) {
                skip += 1;
            }
        }
    }

    /// An item the mouse carried goes back into the pack when no panel is left to put it in.
    fn after_close(&mut self) {
        if !self.any_open() {
            self.inv.put_cursor_away();
            self.tree.hover = None;
            self.tree.pressed = None;
        }
        self.refresh();
    }

    /// A spell of a weapon or of the gear puts something on the character for some seconds (see `procs`).
    pub fn proc_timed(&mut self, what: crate::consume::Timed, seconds: f32) {
        if self.consumed.add_timed(what, seconds) {
            self.stats_dirty = true;
            self.refresh();
        }
    }

    /// Whether the stats changed since the last call without a change of what is worn.
    pub fn take_stats_dirty(&mut self) -> bool {
        std::mem::take(&mut self.stats_dirty)
    }

    /// Whether items were worn or taken off since the last call: the player's loadout needs updating.
    pub fn take_equipment_change(&mut self) -> bool {
        std::mem::take(&mut self.equipment_changed)
    }

    // ---- filling the frames

    pub fn string(&self, key: &str) -> String {
        self.tree.string(key)
    }

    /// Set everything the frames show from the inventory and the stats.
    pub fn refresh(&mut self) {
        self.fill_trade();
        if self.consumed.weapon_changed(self.inv.equipment[crate::items::MAIN_HAND].map(|s| s.item)) {
            self.stats_dirty = true;
        }
        let mut timed = self.consumed.timed_effects();
        timed.extend(self.enchant_effects());
        self.stats = Stats::of_with(&self.inv.equipment, self.progress.level, &self.consumed.boosts(), &timed, &self.talents.perks());
        self.skills.set_bonus(self.stats.skill_bonus);
        self.stats.armor += self.skills.armor_bonus(&self.inv.equipment);
        let [health, mana, stamina] = self.spent_bonus();
        (self.stats.max_health, self.stats.max_mana, self.stats.max_stamina) = (self.stats.max_health + health, self.stats.max_mana + mana, self.stats.max_stamina + stamina);
        let s = self.stats;
        let t = &mut self.tree;
        // The head of the frame.
        t.set_text("CharacterNameText", &self.name);
        t.set_text("CharacterLevelText", &format!("Level {} {} {CLASS}", s.level, self.race.name()));

        // The five attributes and armor, the melee and ranged blocks, the resistances.
        for i in 0..5 {
            let label = format!("{}:", t.string(&format!("SPELL_STAT{i}_NAME")));
            t.set_text(&format!("CharacterStatFrame{}Label", i + 1), &label);
            t.set_text(&format!("CharacterStatFrame{}StatText", i + 1), &s.attributes[i].to_string());
        }
        let armor = t.string("ARMOR_COLON");
        let (melee, damage_label, power_label, ranged) = (t.string("MELEE_ATTACK"), t.string("DAMAGE_COLON"), t.string("ATTACK_POWER_COLON"), t.string("RANGED_ATTACK"));
        t.set_text("CharacterArmorFrameLabel", &armor);
        t.set_text("CharacterArmorFrameStatText", &s.armor.to_string());
        t.set_text("CharacterAttackFrameLabel", &melee);
        t.set_text("CharacterAttackFrameStatText", &s.attack.to_string());
        t.set_text("CharacterDamageFrameLabel", &damage_label);
        let (low, high) = ((s.damage.0.floor() as i32).max(1), (s.damage.1.ceil() as i32).max(1));
        t.set_text("CharacterDamageFrameStatText", &if low < 100 && high < 100 { format!("{low} - {high}") } else { format!("{low}-{high}") });
        t.set_text("CharacterAttackPowerFrameLabel", &power_label);
        t.set_text("CharacterAttackPowerFrameStatText", &s.attack_power.to_string());
        // The ranged block: with a bow, crossbow or gun the skill, the damage and the attack power; else not applicable.
        let n_a = t.string("NOT_APPLICABLE");
        let (r_attack, r_damage, r_power) = match s.ranged {
            Some((min, max, _)) => {
                let (low, high) = ((min.floor() as i32).max(1), (max.ceil() as i32).max(1));
                (s.attack.to_string(), if low < 100 && high < 100 { format!("{low} - {high}") } else { format!("{low}-{high}") }, s.ranged_attack_power.to_string())
            }
            None => (n_a.clone(), n_a.clone(), n_a),
        };
        for (frame, label, text) in [("RangedAttack", ranged.as_str(), r_attack), ("RangedDamage", damage_label.as_str(), r_damage), ("RangedAttackPower", power_label.as_str(), r_power)] {
            t.set_text(&format!("Character{frame}FrameLabel"), label);
            t.set_text(&format!("Character{frame}FrameStatText"), &text);
        }
        // The resistance frames are arcane, fire, nature, frost and shadow, in that order.
        for (i, school) in [5, 1, 2, 3, 4].into_iter().enumerate() {
            t.set_text(&format!("MagicResText{}", i + 1), &s.resist[school].to_string());
        }

        // The equipment slots: the item, or the picture of what goes there.
        for slot in 0..SLOTS {
            let icon = format!("{}IconTexture", SLOT_BUTTONS[slot]);
            let count = format!("{}Count", SLOT_BUTTONS[slot]);
            match self.inv.equipment[slot] {
                Some(stack) => {
                    t.set_texture(&icon, Some(crate::items::get(stack.item).icon));
                    t.set_region_hidden(&count, true);
                }
                None => {
                    t.set_texture(&icon, Some(SLOT_BACKGROUNDS[slot]));
                    t.set_region_hidden(&count, true);
                }
            }
        }

        // Money: the denominations the character has (copper always).
        t.set_money(&format!("{BACKPACK}MoneyFrame"), self.inv.money, false, true);
        t.set_money("MerchantMoneyFrame", self.inv.money, true, true);
        t.set_money("ClassTrainerMoneyFrame", self.inv.money, true, true);

        // The backpack.
        self.bag_scroll = self.bag_scroll.min(self.inv.bag.len().saturating_sub(BACKPACK_SIZE));
        for slot in 0..BACKPACK_SIZE {
            let button = bag_button(slot);
            let (icon, count) = (format!("{button}IconTexture"), format!("{button}Count"));
            match self.inv.bag[slot + self.bag_scroll] {
                Some(stack) => {
                    t.set_texture(&icon, Some(crate::items::get(stack.item).icon));
                    t.set_text(&count, &stack.count.to_string());
                    t.set_region_hidden(&count, stack.count <= 1);
                }
                None => {
                    t.set_texture(&icon, None);
                    t.set_region_hidden(&count, true);
                }
            }
        }
    }

    // ---- the mouse

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

    // ---- tooltips

    fn item_lines(&self, stack: Stack) -> Vec<TipLine> {
        let mut lines = self.item_lines_plain(stack);
        // What was put on it with Enchanting, in green.
        if let Some(e) = content::get().enchants.get(&stack.enchant) {
            lines.push(TipLine::new(e.name.clone(), GREEN));
        }
        lines
    }

    fn item_lines_plain(&self, stack: Stack) -> Vec<TipLine> {
        let item: &ItemDef = crate::items::get(stack.item);
        let q = QUALITY_COLORS[item.quality];
        let mut lines = vec![TipLine::new(item.name, [q[0], q[1], q[2], 1.0])];
        // How it binds, whether there can be only one, and a conjured item that is gone in time.
        let bind = match item.bind {
            crate::items::Bind::Pickup => Some("ITEM_BIND_ON_PICKUP"),
            crate::items::Bind::Equip => Some("ITEM_BIND_ON_EQUIP"),
            crate::items::Bind::Use => Some("ITEM_BIND_ON_USE"),
            crate::items::Bind::Quest => Some("ITEM_BIND_QUEST"),
            crate::items::Bind::Never => item.quest.then_some("ITEM_BIND_QUEST"),
        };
        if let Some(key) = bind {
            let text = self.tree.string(key);
            lines.push(TipLine::new(if text.is_empty() { "Quest Item".to_string() } else { text }, WHITE));
        }
        if item.unique {
            lines.push(TipLine::new(self.tree.string("ITEM_UNIQUE"), WHITE));
        }
        if item.flags & 2 != 0 {
            lines.push(TipLine::new(self.tree.string("ITEM_CONJURED"), WHITE));
        }
        if item.slots > 0 {
            lines.push(TipLine::new(self.tree.string("CONTAINER_SLOTS").replacen("%d", &item.slots.to_string(), 1).replacen("%s", if item.subclass == 0 { "Bag" } else { "Quiver" }, 1), WHITE));
        }
        if !item.kind.0.is_empty() {
            lines.push(TipLine::double(item.kind.0, item.kind.1, WHITE, WHITE));
        }
        if let Some(w) = item.weapon {
            lines.push(TipLine::double(format!("{:.0} - {:.0} Damage", w.min, w.max), format!("Speed {:.2}", w.speed), WHITE, WHITE));
            for &(low, high, school) in item.more_damage {
                lines.push(TipLine::new(format!("+ {:.0} - {:.0} {} Damage", low, high, SCHOOLS[(school as usize).min(6)]), WHITE));
            }
            let dps = (w.min + w.max + item.more_damage.iter().map(|d| d.0 + d.1).sum::<f32>()) / 2.0 / w.speed;
            lines.push(TipLine::new(format!("({dps:.1} damage per second)"), WHITE));
        }
        if item.armor > 0 {
            lines.push(TipLine::new(format!("{} Armor", item.armor), WHITE));
        }
        if item.block > 0 {
            lines.push(TipLine::new(format!("{} Block", item.block), WHITE));
        }
        for (name, bonus) in ["Strength", "Agility", "Stamina", "Intellect", "Spirit"].iter().zip(item.stats) {
            if bonus != 0 {
                lines.push(TipLine::new(format!("{}{} {name}", if bonus > 0 { "+" } else { "-" }, bonus.abs()), WHITE));
            }
        }
        for (name, bonus) in [("Health", item.health), ("Mana", item.mana)] {
            if bonus != 0 {
                lines.push(TipLine::new(format!("{}{} {name}", if bonus > 0 { "+" } else { "-" }, bonus.abs()), WHITE));
            }
        }
        for (school, bonus) in item.resist.iter().enumerate() {
            if *bonus != 0 {
                lines.push(TipLine::new(format!("{}{} {} Resistance", if *bonus > 0 { "+" } else { "-" }, bonus.abs(), SCHOOLS[school + 1]), WHITE));
            }
        }
        if item.weight > 0.0 {
            lines.push(TipLine::new(format!("Weight: {:.2}", item.weight), WHITE));
        }
        if item.durability > 0 {
            lines.push(TipLine::new(self.tree.string("DURABILITY_TEMPLATE").replacen("%d", &item.durability.to_string(), 2), WHITE));
        }
        // Who it is for, and what is asked of whoever uses it (red when this character falls short).
        if item.classes != 0 {
            let names: Vec<&str> = CLASSES.iter().filter(|(bit, _)| item.classes & bit != 0).map(|(_, n)| *n).collect();
            lines.push(TipLine::new(self.tree.string("ITEM_CLASSES_ALLOWED").replacen("%s", &names.join(", "), 1), WHITE));
        }
        if item.races != 0 {
            let names: Vec<&str> = RACES.iter().filter(|(bit, _)| item.races & bit != 0).map(|(_, n)| *n).collect();
            lines.push(TipLine::new(self.tree.string("ITEM_RACES_ALLOWED").replacen("%s", &names.join(", "), 1), if item.races & self.race.mask() != 0 { WHITE } else { RED }));
        }
        if item.level > 1 {
            lines.push(TipLine::new(self.tree.string("ITEM_MIN_LEVEL").replacen("%d", &item.level.to_string(), 1), if self.progress.level >= item.level { WHITE } else { RED }));
        }
        if item.skill.0 != 0 {
            lines.push(TipLine::new(self.tree.string("ITEM_REQ_SKILL").replacen("%s", &format!("{} ({})", skill_name(item.skill.0), item.skill.1), 1), RED));
        }
        if let Some(spell) = item.tome {
            let def = content::spell(spell);
            if def.level > 1 {
                lines.push(TipLine::new(format!("Requires {} ({})", def.class.name(), def.level), if self.skills.rank(def.class) >= def.level { WHITE } else { RED }));
            }
            lines.push(TipLine::new(format!("Use: Teaches you {}.", def.name), [0.12, 1.0, 0.0, 1.0]));
        }
        // What the original's spells do when it is used, worn or lands a blow, in the words of the spell.
        for s in item.spells {
            let trigger = match s.trigger {
                0 | 4 | 5 => "ITEM_SPELL_TRIGGER_ONUSE",
                1 => "ITEM_SPELL_TRIGGER_ONEQUIP",
                2 => "ITEM_SPELL_TRIGGER_ONPROC",
                _ => continue,
            };
            if !s.text.is_empty() {
                lines.push(TipLine::new(format!("{} {}", self.tree.string(trigger), s.text.replace("$z", "Northshire Valley")), [0.12, 1.0, 0.0, 1.0]));
            }
        }
        if !item.text.is_empty() {
            lines.push(TipLine::new(format!("\"{}\"", item.text), [1.0, 0.82, 0.0, 1.0]));
        }
        lines
    }

    /// The tooltip of a stat of the character sheet: the original's (the stat and the class's words on it), and under it what the game works
    /// out of it (the original shows none of these numbers; its 1.12 sheet has no lines for them).
    fn stat_lines(&self, name: &str) -> Option<Vec<TipLine>> {
        let s = self.stats;
        let level = self.progress.level;
        let head = |text: String| TipLine::new(text, WHITE);
        let note = |text: String| TipLine::new(text, GOLD);
        let more = |text: String| TipLine::new(text, WHITE);
        if let Some(i) = name.strip_prefix("CharacterStatFrame").and_then(|n| n.chars().next()?.to_digit(10)).filter(|i| (1..=5).contains(i)) {
            let i = i as usize - 1;
            let stat = ["STRENGTH", "AGILITY", "STAMINA", "INTELLECT", "SPIRIT"][i];
            let words = self.string(&format!("PALADIN_{stat}_TOOLTIP"));
            let words = if words.starts_with("PALADIN_") { self.string(&format!("DEFAULT_{stat}_TOOLTIP")) } else { words };
            let mut lines = vec![head(format!("{} {}", self.string(&format!("SPELL_STAT{i}_NAME")), s.attributes[i])), note(words)];
            match i {
                0 => {
                    lines.push(more(format!("Melee attack power: {}", s.attack_power)));
                    if s.block > 0.0 {
                        lines.push(more(format!("Block value: {:.0}", s.block_value)));
                    }
                }
                1 => lines.extend([
                    more(format!("Critical strike chance: {:.2}%", s.crit)),
                    more(format!("Dodge chance: {:.2}%", s.dodge)),
                    more(format!("Ranged attack power: {}", s.ranged_attack_power)),
                ]),
                2 => lines.push(more(format!("Maximum health: {:.0}", s.max_health))),
                3 => {
                    let (low, high) = s.spell_crit[1..].iter().fold((f32::MAX, 0.0f32), |(l, h), c| (l.min(*c), h.max(*c)));
                    let crit = if high - low < 0.005 { format!("{low:.2}%") } else { format!("{low:.2}% to {high:.2}% by school") };
                    lines.extend([more(format!("Spell critical strike chance: {crit}")), more(format!("Maximum mana: {:.0}", s.max_mana))]);
                    if s.healing > 0 || s.spell_damage.iter().any(|d| *d > 0) {
                        lines.push(more(format!("Healing done: +{}; holy damage done: +{}", s.healing, s.spell_damage[1])));
                    }
                    if s.spell_hit != 0.0 {
                        lines.push(more(format!("Chance to hit with spells: +{:.0}%", s.spell_hit)));
                    }
                }
                _ => lines.extend([
                    more(format!("Health regeneration: {:.1} per second out of combat", s.health_regen)),
                    more(format!("Mana regeneration: {:.1} per second ({:.1} just after a spell)", s.mana_regen, s.mana_regen_casting)),
                ]),
            }
            return Some(lines);
        }
        Some(match name {
            "CharacterArmorFrame" => {
                let reduction = 100.0 * s.mitigation();
                let words = self.string("ARMOR_TOOLTIP").replacen("%d", &level.to_string(), 1).replacen("%.1f%%", &format!("{reduction:.1}%"), 1);
                vec![
                    head(format!("{} {}", self.string("ARMOR"), s.armor)),
                    note(words),
                    more(format!("Dodge chance: {:.2}%", s.dodge)),
                    more(format!("Parry chance: {:.2}%", s.parry)),
                    more(format!("Block chance: {:.2}%", s.block)),
                ]
            }
            "CharacterAttackFrame" => vec![
                head(self.string("ATTACK_TOOLTIP")),
                note(self.string("ATTACK_TOOLTIP_SUBTEXT")),
                more(format!("Critical strike chance: {:.2}%", s.crit)),
                more(format!("Chance to miss a creature of your level: {:.1}%", crate::levels::melee_miss(level, level, s.hit))),
            ],
            "CharacterAttackPowerFrame" => vec![
                head(format!("{} {}", self.string("MELEE_ATTACK_POWER"), s.attack_power)),
                note(self.string("MELEE_ATTACK_POWER_TOOLTIP").replacen("%.1f", &format!("{:.1}", s.attack_power as f32 / crate::items::ATTACK_POWER_PER_DPS), 1)),
            ],
            "CharacterRangedAttackPowerFrame" if s.ranged.is_some() => vec![
                head(format!("{} {}", self.string("RANGED_ATTACK_POWER"), s.ranged_attack_power)),
                note(self.string("RANGED_ATTACK_POWER_TOOLTIP").replacen("%.1f", &format!("{:.1}", s.ranged_attack_power as f32 / crate::items::ATTACK_POWER_PER_DPS), 1)),
            ],
            _ => return None,
        })
    }

    /// The tooltip for the frame under the mouse, if it has one.
    fn tooltip(&self) -> Option<Vec<TipLine>> {
        let hover = self.tree.hover?;
        let name = self.tree.name(hover);
        if let Some(action) = self.book_entry_of(name) {
            return Some(self.action_lines(action));
        }
        if let Some(lines) = self.talent_lines(name).or_else(|| self.trade_lines(name)).or_else(|| self.trainer_lines(name)) {
            return Some(lines);
        }
        if let Some(slot) = SLOT_BUTTONS.iter().position(|b| *b == name) {
            return Some(match self.inv.equipment[slot] {
                Some(stack) => self.item_lines(stack),
                // An empty slot names itself: `HEADSLOT` is "Head".
                None => vec![TipLine::new(self.string(&name["Character".len()..].to_uppercase()), WHITE)],
            });
        }
        if let Some(i) = name.strip_prefix("LootButton").and_then(|n| n.parse::<usize>().ok()) {
            let (_, loot) = self.loot.as_ref()?;
            let k = (*Self::loot_slots(loot).get(i - 1)?)?;
            return Some(self.item_lines(loot.items[k]));
        }
        if let Some(i) = name.strip_prefix("MerchantItem").and_then(|n| n.strip_suffix("ItemButton")).and_then(|n| n.parse::<usize>().ok()) {
            let &item = self.merchant?.1.vendor.get(self.merchant_page * MERCHANT_ITEMS + i - 1)?;
            return Some(self.item_lines(Stack { item, count: 1, enchant: 0 }));
        }
        if let Some(i) = name.strip_prefix("QuestLogItem").and_then(|n| n.parse::<usize>().ok()) {
            let quest = self.quests.active.get(self.log_selected)?.quest;
            return Some(self.item_lines(*content::quest(quest).rewards().nth(i - 1)?));
        }
        if let Some((_, _, quest, _)) = self.questing {
            let q = content::quest(quest);
            for prefix in ["QuestDetailItem", "QuestRewardItem", "QuestProgressItem"] {
                if let Some(i) = name.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok()) {
                    let stack = if prefix == "QuestProgressItem" {
                        q.goals.iter().filter_map(|g| if let Goal::Collect { item, count } = *g { Some(Stack { item, count, enchant: 0 }) } else { None }).nth(i - 1)?
                    } else {
                        *q.rewards().nth(i - 1)?
                    };
                    return Some(self.item_lines(stack));
                }
            }
        }
        if name == "MerchantBuyBackItemItemButton" {
            return self.buyback.map(|(stack, _)| self.item_lines(stack));
        }
        if let Some(lines) = self.stat_lines(name) {
            return Some(lines);
        }
        let j = name.strip_prefix(&format!("{BACKPACK}Item"))?.parse::<usize>().ok()?;
        let stack = self.inv.bag[self.bag_index(j)]?;
        let mut lines = self.item_lines(stack);
        // With a merchant open, the tooltip also says what he would pay.
        if self.merchant.is_some() {
            let price = crate::items::get(stack.item).sell * stack.count;
            lines.push(match price {
                0 => TipLine::new("No sell price", WHITE),
                p => TipLine::new(format!("Sell Price: {}", coins(p)), WHITE),
            });
        }
        Some(lines)
    }

    // ---- the pictures

    /// Aim the doll and portrait cameras at the player and draw the character's model into them.
    pub fn render_pictures(&mut self, renderer: &mut Renderer, gpu: &Gpu, player: &Player, world: &World) {
        if !self.character_open() && !self.talents_open() && !self.trade_open() {
            return;
        }
        let (feet, facing) = (player.feet(), player.facing());
        let ids = player.entities();
        // In front of the character, turned by the rotate buttons: 5.2 yards away at the height of its middle.
        let around = facing + std::f32::consts::PI + self.doll_turn;
        let (sy, cy) = (around.sin(), around.cos());
        let mut doll = Camera::new(feet + Vec3::new(-sy * 5.2, 1.0, cy * 5.2), around, 0.0);
        (doll.fov_y, doll.aspect, doll.near, doll.far) = (25f32.to_radians(), DOLL_SIZE.0 as f32 / DOLL_SIZE.1 as f32, 0.1, 80.0);
        renderer.render_offscreen(gpu, self.doll.0, &doll, &ids, world.tiles());
        // The portrait is the face, straight on, from a yard and a quarter.
        let front = facing + std::f32::consts::PI;
        let (sf, cf) = (front.sin(), front.cos());
        let mut face = Camera::new(feet + Vec3::new(-sf * 1.25, 1.84, cf * 1.25), front, 0.0);
        (face.fov_y, face.aspect, face.near, face.far) = (30f32.to_radians(), 1.0, 0.1, 40.0);
        renderer.render_offscreen(gpu, self.portrait.0, &face, &ids, world.tiles());
    }

    /// Queue the messages that show whether or not a panel is open: the error at the top and a `hint` low on the screen.
    pub fn draw_overlay(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), dt: f32, hint: Option<&str>) {
        let mount = self.mount.clone();
        // `UIErrorsFrame` sits 122 units below the top, its lines stacked downward; the text is `ErrorFont`.
        self.update_cast_bar(dt);
        for (_, _, age) in &mut self.messages {
            *age += dt;
        }
        self.messages.retain(|(_, _, age)| *age < ERROR_SECONDS);
        let s = screen.1 as f32 / ui_height();
        for (i, (text, color, age)) in self.messages.clone().into_iter().enumerate() {
            let alpha = if age < ERROR_SECONDS - ERROR_FADE { 1.0 } else { ((ERROR_SECONDS - age) / ERROR_FADE).max(0.0) };
            self.tree.draw_text_aligned(ui, gpu, &mount, screen, "ErrorFont", &text, (screen.0 as f32 / 2.0, (122.0 + 20.0 * i as f32) * s), 0.5, [color[0], color[1], color[2], alpha]);
        }
        if self.breath_shown {
            // The label of the breath bar, at the top of `MirrorTimer1` (96 units below the top of the screen).
            self.tree.draw_text_aligned(ui, gpu, &self.mount.clone(), screen, "GameFontHighlight", "Breath", (screen.0 as f32 / 2.0, 96.0 * s), 0.5, WHITE);
        }
        self.draw_tracker(ui, gpu, screen);
        self.draw_trade(ui, gpu, screen);
        self.draw_load(ui, gpu, screen);
        if let Some(hint) = hint {
            self.tree.draw_text_centered(ui, gpu, &mount, screen, "GameFontHighlight", hint, 190.0, WHITE);
        }
    }

    /// What the character carries out of what it can, under the title of the open backpack (red when it is slowed).
    fn draw_load(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        let Some(frame) = self.backpack_open().then(|| self.tree.rect_of(BACKPACK, screen)).flatten() else { return };
        let (mount, s, (weight, capacity)) = (self.mount.clone(), screen.1 as f32 / ui_height(), self.load());
        let at = ((frame.x + frame.w - 10.0 * CONTAINER_SCALE) * s, screen.1 as f32 - (frame.y + frame.h - 30.0 * CONTAINER_SCALE) * s);
        let color = if weight > capacity { RED } else { WHITE };
        self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlightSmall", &format!("Weight: {weight:.1} / {capacity:.0}"), at, 1.0, color);
    }

    /// The quests being worked on, at the top right of the screen like `QuestWatchFrame`: the title and a line for each goal.
    fn draw_tracker(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        let mount = self.mount.clone();
        let s = screen.1 as f32 / ui_height();
        let (x, mut y) = (screen.0 as f32 - 24.0 * s, 160.0 * s);
        let step = 15.0 * s;
        for active in self.quests.active.clone() {
            let q = content::quest(active.quest);
            self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormal", &q.title, (x, y), 1.0, GOLD);
            y += step;
            if q.goals.is_empty() {
                self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlightSmall", &format!(" - {}", q.objective), (x, y), 1.0, WHITE);
                y += step;
            }
            for (g, goal) in q.goals.iter().enumerate() {
                let (name, need) = match *goal {
                    Goal::Kill { creature, count } => (format!("{} slain", content::get().creatures[creature].name), count),
                    Goal::Collect { item, count } => (crate::items::get(item).name.to_string(), count),
                    Goal::Reach { .. } => ("Destination".to_string(), 1),
                };
                let have = self.quests.have(active.quest, g, &self.inv);
                let color = if have >= need { GREY } else { WHITE };
                self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlightSmall", &format!(" - {name}: {have}/{need}"), (x, y), 1.0, color);
                y += step;
            }
            y += 5.0 * s;
        }
    }

    /// The wheel turned over a window: its scroll frame moves. False when there is nothing there to scroll.
    pub fn scroll(&mut self, pos: (f32, f32), screen: (u32, u32), lines: f32) -> bool {
        if self.map_open {
            self.map_zoom(pos, screen, lines);
            return true;
        }
        if self.keys_open() {
            self.keys_wheel(lines);
            return true;
        }
        if self.sleep_step(lines.signum()) {
            return true;
        }
        self.any_open() && (self.backpack_wheel(pos, screen, lines) || self.skills_wheel(lines) || self.trade_wheel(lines) || self.trainer_wheel(lines) || self.tree.scroll_at(pos, screen, lines))
    }

    /// The wheel over the backpack moves its window a row up or down the list of slots.
    fn backpack_wheel(&mut self, pos: (f32, f32), screen: (u32, u32), lines: f32) -> bool {
        let over = self.backpack_open() && self.tree.hit_test(pos, screen).is_some_and(|h| self.tree.name(h).starts_with(BACKPACK));
        if over && lines != 0.0 {
            let last = self.inv.bag.len().saturating_sub(BACKPACK_SIZE);
            self.bag_scroll = if lines > 0.0 { self.bag_scroll.saturating_sub(CONTAINER_COLUMNS) } else { (self.bag_scroll + CONTAINER_COLUMNS).min(last) };
            self.refresh();
        }
        over
    }

    /// Show the scroll bar of every scroll frame that has more to see, with its knob where the frame is scrolled to.
    fn update_scroll_bars(&mut self, screen: (u32, u32)) {
        for frame in SCROLL_FRAMES {
            let bar = format!("{frame}ScrollBar");
            let state = self.tree.scroll_state(frame, screen).filter(|_| self.tree.is_name_shown(frame));
            self.tree.show(&bar, state.is_some_and(|(_, max)| max > 0.0));
            if let (Some((at, max)), Some(track)) = (state.filter(|(_, max)| *max > 0.0), self.tree.rect_of(&bar, screen)) {
                // The knob is 16 units and travels the length of the bar less its own.
                let travel = (track.h - 16.0).max(0.0);
                self.tree.set_region_point(&format!("{bar}ThumbTexture"), Point::TopLeft, Some(&bar), Some(Point::TopLeft), 0.0, -travel * at / max);
            }
        }
    }

    /// Show the target (`TargetFrame_Update`): its name, level in the colour of how hard it is, health, and the colour of its
    /// name plate; nothing hides the frame. The hostile ones are red, the neutral ones yellow, the friendly ones green.
    pub fn set_target(&mut self, view: Option<crate::creature::TargetView>) {
        // Whoever is talked to shows his face in the portrait of the talk, trade and quest windows (the book otherwise).
        let face = view.as_ref().filter(|v| self.talking_to() == Some(v.id)).map(|_| self.target_portrait.1);
        for portrait in ["GossipFramePortrait", "MerchantFramePortrait", "QuestFramePortrait", "ClassTrainerFramePortrait"] {
            self.tree.set_region_texture(portrait, face);
        }
        let Some(v) = view else {
            if self.target.take().is_some() {
                self.tree.show("TargetFrame", false);
                self.events.push(Sfx::Named("INTERFACESOUND_LOSTTARGETUNIT"));
            }
            return;
        };
        if self.target != Some(v.id) {
            self.events.push(Sfx::Named(if v.hostile || v.neutral { "igCreatureAggroSelect" } else { "igCharacterNPCSelect" }));
        }
        self.target = Some(v.id);
        self.tree.show("TargetFrame", true);
        self.tree.set_text("TargetName", &v.name);
        let level = self.progress.level;
        let skull = levels::is_skull(level, v.level);
        self.tree.set_region_hidden("TargetHighLevelTexture", !skull);
        self.tree.set_region_hidden("TargetLevelText", skull);
        self.tree.set_text("TargetLevelText", &v.level.to_string());
        self.tree.set_text_color("TargetLevelText", if v.hostile || v.neutral { levels::difficulty_color(level, v.level) } else { GOLD });
        self.tree.set_region_hidden("TargetDeadText", !v.dead);
        self.tree.set_region_tint("TargetFrameNameBackground", if v.hostile { [1.0, 0.0, 0.0, 1.0] } else if v.neutral { [1.0, 1.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] });
        self.tree.set_bar("TargetFrameHealthBar", v.fraction, Some([0.0, 1.0, 0.0, 1.0]));
    }

    /// Say which spells of creatures have hold of the character (every frame): their icons are shown with the buffs.
    pub fn set_afflictions(&mut self, list: &[crate::foes::Affliction]) {
        self.afflicted = list.iter().map(|a| (format!(r"Interface\Icons\{}", a.icon), a.left)).collect();
    }

    /// Show the buffs as the icons of `BuffFrame`.
    fn update_buffs(&mut self) {
        let mut buffs: Vec<(String, f32)> = self.consumed.buffs.iter().map(|b| (crate::items::get(b.item).icon.to_string(), b.left)).collect();
        buffs.extend(self.afflicted.iter().cloned());
        self.tree.show("BuffFrame", !buffs.is_empty());
        let durations = self.show_durations();
        for i in 0..24 {
            let button = format!("BuffButton{i}");
            match buffs.get(i) {
                Some((icon, left)) => {
                    self.tree.show(&button, true);
                    self.tree.set_texture(&format!("{button}Icon"), Some(icon));
                    self.tree.set_region_hidden(&format!("{button}Duration"), !durations);
                    let lasts = left.ceil() as u32;
                    let text = match lasts {
                        3600.. => self.string("HOUR_ONELETTER_ABBR").replace("%d", &(lasts / 3600).to_string()),
                        60.. => self.string("MINUTE_ONELETTER_ABBR").replace("%d", &(lasts / 60).to_string()),
                        _ => self.string("SECOND_ONELETTER_ABBR").replace("%d", &lasts.to_string()),
                    };
                    self.tree.set_text(&format!("{button}Duration"), &text);
                }
                None => self.tree.show(&button, false),
            }
        }
    }

    pub fn target_shown(&self) -> bool {
        self.target.is_some()
    }

    /// Draw the target's head into the portrait of the target frame.
    pub fn render_target(&mut self, renderer: &mut Renderer, gpu: &Gpu, creatures: &crate::creature::Creatures, world: &World) {
        let Some(shot) = self.target.and_then(|t| creatures.portrait_shot(t)) else { return };
        renderer.render_offscreen(gpu, self.target_portrait.0, &shot.0, &shot.1, world.tiles());
    }

    /// Lay the open panels out for `screen` (what drawing does first), for clicks that come before the first draw.
    pub fn layout(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        let mount = self.mount.clone();
        self.tree.measure(ui, gpu, &mount, screen);
    }

    /// Queue the names floating over creatures (`UnitName`: the game font, tinted by how the creature feels about you).
    pub fn draw_labels(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), labels: &[crate::creature::Label]) {
        let mount = self.mount.clone();
        for l in labels {
            // The name stands one em above the subname, which stands on the point.
            let name_y = if l.sub.is_some() { l.y - l.line } else { l.y };
            self.tree.draw_text_world(ui, gpu, &mount, screen, "GameFontNormalLarge", &l.text, (l.x, name_y), l.line, l.color);
            if let Some(sub) = &l.sub {
                self.tree.draw_text_world(ui, gpu, &mount, screen, "GameFontNormalLarge", sub, (l.x, l.y), l.line, l.color);
            }
        }
    }

    /// Queue the open panels, the carried item and the tooltip for drawing.
    pub fn draw(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        self.update_buffs();
        if !self.any_open() && !self.target_shown() && !self.cast_bar_visible() && !self.breath_shown && self.consumed.buffs.is_empty() && self.afflicted.is_empty() {
            return;
        }
        let mount = self.mount.clone();
        self.tree.measure(ui, gpu, &mount, screen);
        self.update_scroll_bars(screen);
        if self.popup_open() {
            // The question is as tall as its text and one row of buttons (`StaticPopup_Resize`).
            self.tree.measure(ui, gpu, &mount, screen);
            let room = self.sleep_room();
            let height = 16.0 + self.tree.text_height("StaticPopup1Text") + room + 8.0 + 20.0 + 16.0;
            self.tree.set_size(POPUP, None, Some(height));
            self.tree.set_point("StaticPopup1Button1", Point::TopRight, Some("StaticPopup1Text"), Some(Point::Bottom), -6.0, -8.0 - room);
        }
        self.tree.resize_tab(ui, gpu, &mount, "CharacterFrameTab1", 0.0);
        self.tree.resize_tab(ui, gpu, &mount, "CharacterFrameTab4", 0.0);
        self.tree.resize_tab(ui, gpu, &mount, "MerchantFrameTab1", 0.0);
        for tab in 1..=3 {
            self.tree.resize_tab(ui, gpu, &mount, &format!("TalentFrameTab{tab}"), 0.0);
        }
        if self.menu_open() || self.options_open() {
            // The world dims behind the menu, as behind the map and the level-up panel.
            levelup::quad(ui, crate::ui::WHITE, [0.0, 0.0, screen.0 as f32, screen.1 as f32], [0.0, 0.0, 0.0, 0.38], false);
        }
        self.tree.draw(ui, gpu, &mount, screen);
                let s =screen.1 as f32 / ui_height();
        // The carried item hangs off the mouse.
        if let (Some(c), Some((x, y))) = (self.inv.cursor, self.cursor) {
            let size = 37.0 * s;
            self.tree.draw_icon(ui, gpu, &mount, crate::items::get(c.item).icon, [x - size / 2.0, y - size / 2.0, size, size]);
        } else if let (Some(lines), Some(hover)) = (self.tooltip(), self.tree.hover) {
            if let Some(owner) = self.tree.rect_of(&self.tree.name(hover).to_string(), screen) {
                self.tree.draw_tooltip(ui, gpu, &mount, screen, owner, &lines);
            }
        }
    }
}

/// The index of a creature kind (its place in the content's creatures).
fn kind_of(def: &CreatureDef) -> usize {
    content::get().creatures.iter().position(|c| std::ptr::eq(c, def)).expect("a creature kind of the content")
}

/// An amount of copper as the original's coin text: `1g 20s 5c`.
fn coins(copper: u32) -> String {
    [(copper / 10000, 'g'), (copper / 100 % 100, 's'), (copper % 100, 'c')].iter().filter(|(n, _)| *n > 0).map(|(n, unit)| format!("{n}{unit}")).collect::<Vec<_>>().join(" ")
}

/// The picture of a heap of coins: copper, silver or gold, in a small or a big pile.
fn coin_icon(copper: u32) -> &'static str {
    match copper {
        0..=9 => r"Interface\Icons\INV_Misc_Coin_05",
        10..=99 => r"Interface\Icons\INV_Misc_Coin_06",
        100..=999 => r"Interface\Icons\INV_Misc_Coin_03",
        1000..=9999 => r"Interface\Icons\INV_Misc_Coin_04",
        10000..=99999 => r"Interface\Icons\INV_Misc_Coin_01",
        _ => r"Interface\Icons\INV_Misc_Coin_02",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_read_like_the_originals_coins() {
        assert_eq!((coins(5), coins(120), coins(10000), coins(10105)), ("5c".to_string(), "1s 20c".to_string(), "1g".to_string(), "1g 1s 5c".to_string()));
    }

    #[test]
    fn loot_slots_list_the_money_first_then_the_items() {
        let loot = Loot { copper: 7, items: vec![Stack { item: 7, count: 1, enchant: 0 }, Stack { item: 8, count: 1, enchant: 0 }] };
        assert_eq!(Panels::loot_slots(&loot), [None, Some(0), Some(1)]);
        let no_money = Loot { copper: 0, ..loot };
        assert_eq!(Panels::loot_slots(&no_money), [Some(0), Some(1)]);
    }

    #[test]
    fn backpack_buttons_are_numbered_from_the_bottom_right() {
        assert_eq!((bag_button(0), bag_button(15)), ("ContainerFrame1Item16".to_string(), "ContainerFrame1Item1".to_string()));
    }

    #[test]
    fn every_frame_and_picture_the_panels_use_exists_in_the_game_files() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let mut tree = Tree::new(Rc::new(Registry::load(&mount).unwrap()));
        tree.instantiate(CHARACTER).unwrap();
        tree.instantiate(BACKPACK).unwrap();
        // The 20 equipment slot buttons and the 16 backpack buttons, each with the icon and count regions.
        for name in SLOT_BUTTONS.iter().map(|s| s.to_string()).chain((0..BACKPACK_SIZE).map(bag_button)) {
            assert!(tree.index(&name).is_some(), "{name} is missing");
            assert!(tree.region_rect_of(&format!("{name}IconTexture"), (1024, 768)).is_some(), "{name} has no icon region");
            assert!(tree.region_rect_of(&format!("{name}Count"), (1024, 768)).is_some(), "{name} has no count region");
        }
        // Every text the sheet fills in has a region, and every item icon and empty slot picture is a real file.
        for region in ["CharacterNameText", "CharacterLevelText", "CharacterStatFrame5StatText", "CharacterArmorFrameStatText", "CharacterDamageFrameStatText", "MagicResText5", "ContainerFrame1Name", "ContainerFrame1MoneyFrameCopperButtonText"] {
            assert!(tree.region_rect_of(region, (1024, 768)).is_some(), "{region} is missing");
        }
        // The question (`StaticPopup1`) with its text, two buttons and the picture it hides, and the reward highlight.
        tree.instantiate(POPUP).unwrap();
        for name in ["StaticPopup1Text", "StaticPopup1AlertIcon", "StaticPopup1Button1Text", "StaticPopup1Button2Text"] {
            assert!(tree.region_rect_of(name, (1024, 768)).is_some(), "{name} is missing");
        }
        tree.instantiate(QUEST).unwrap();
        assert!(tree.rect_of("QuestRewardItemHighlight", (1024, 768)).is_some());
        // The quest log: its list, details, objectives lines and reward items.
        tree.instantiate(QUESTLOG).unwrap();
        for name in ["QuestLogQuestCount", "QuestLogQuestTitle", "QuestLogObjectivesText", "QuestLogDescriptionTitle", "QuestLogQuestDescription", "QuestLogRewardTitleText", "QuestLogItemReceiveText", "QuestLogItemChooseText", "QuestLogRequiredMoneyText"] {
            assert!(tree.region_rect_of(name, (1024, 768)).is_some(), "{name} is missing");
        }
        for i in 1..=LOG_LINES {
            for part in ["NormalText", "Tag"] {
                assert!(tree.region_rect_of(&format!("QuestLogTitle{i}{part}"), (1024, 768)).is_some(), "QuestLogTitle{i}{part} is missing");
            }
        }
        for i in 1..=LOG_GOALS {
            assert!(tree.region_rect_of(&format!("QuestLogObjective{i}"), (1024, 768)).is_some(), "QuestLogObjective{i} is missing");
        }
        for i in 1..=QUEST_ITEMS {
            for part in ["IconTexture", "Name", "Count"] {
                assert!(tree.region_rect_of(&format!("QuestLogItem{i}{part}"), (1024, 768)).is_some(), "QuestLogItem{i}{part} is missing");
            }
        }
        for button in ["QuestLogFrameAbandonButton", "QuestFrameExitButton", "QuestLogFrameCloseButton"] {
            assert!(tree.rect_of(button, (1024, 768)).is_some(), "{button} is missing");
        }
        // The quest window: the three screens with their texts, buttons and item buttons.
        tree.instantiate(QUEST).unwrap();
        for name in ["QuestFrameNpcNameText", "QuestTitleText", "QuestDescription", "QuestObjectiveText", "QuestDetailObjectiveTitleText", "QuestDetailRewardTitleText", "QuestDetailItemReceiveText", "QuestDetailItemChooseText", "QuestProgressTitleText", "QuestProgressText", "QuestProgressRequiredItemsText", "QuestProgressRequiredMoneyText", "QuestRewardTitleText", "QuestRewardText", "QuestRewardRewardTitleText", "QuestRewardItemReceiveText", "QuestRewardItemChooseText"] {
            assert!(tree.region_rect_of(name, (1024, 768)).is_some(), "{name} is missing");
        }
        for i in 1..=QUEST_ITEMS {
            for prefix in ["QuestDetailItem", "QuestRewardItem"] {
                for part in ["IconTexture", "Name", "Count"] {
                    assert!(tree.region_rect_of(&format!("{prefix}{i}{part}"), (1024, 768)).is_some(), "{prefix}{i}{part} is missing");
                }
            }
        }
        for i in 1..=PROGRESS_ITEMS {
            for part in ["IconTexture", "Name", "Count"] {
                assert!(tree.region_rect_of(&format!("QuestProgressItem{i}{part}"), (1024, 768)).is_some(), "QuestProgressItem{i}{part} is missing");
            }
        }
        for button in ["QuestFrameAcceptButton", "QuestFrameDeclineButton", "QuestFrameCompleteButton", "QuestFrameGoodbyeButton", "QuestFrameCompleteQuestButton", "QuestFrameCancelButton", "QuestFrameCloseButton"] {
            assert!(tree.rect_of(button, (1024, 768)).is_some(), "{button} is missing");
        }
        for icon in ["AvailableQuestIcon", "ActiveQuestIcon", "GossipGossipIcon", "VendorGossipIcon"] {
            assert!(mount.read(&format!(r"Interface\GossipFrame\{icon}.blp")).is_ok(), "{icon} icon");
        }
        // The merchant window: ten wares with names, pictures and prices, the buyback slot, and the coins of a price.
        tree.instantiate(MERCHANT).unwrap();
        for i in 1..=MERCHANT_ITEMS {
            for part in ["Name", "ItemButtonIconTexture", "ItemButtonCount", "MoneyFrameCopperButtonText"] {
                assert!(tree.region_rect_of(&format!("MerchantItem{i}{part}"), (1024, 768)).is_some(), "MerchantItem{i}{part} is missing");
            }
        }
        for name in ["MerchantNameText", "MerchantBuyBackItemName", "MerchantBuyBackItemItemButtonIconTexture", "MerchantBuyBackItemMoneyFrameGoldButtonText"] {
            assert!(tree.region_rect_of(name, (1024, 768)).is_some(), "{name} is missing");
        }
        // The talk window: its name, text, portrait, buttons and the pieces its script hides.
        tree.instantiate(GOSSIP).unwrap();
        for name in ["GossipFrameNpcNameText", "GossipGreetingText", "GossipFramePortrait", "GossipTitleButton1Text", "GossipTitleButton32Text"] {
            assert!(tree.region_rect_of(name, (1024, 768)).is_some(), "{name} is missing");
        }
        // The loot window with its four buttons, their text and count regions, and the coin pictures.
        tree.instantiate(LOOT).unwrap();
        for i in 1..=LOOT_BUTTONS {
            for part in ["IconTexture", "Text", "Count"] {
                assert!(tree.region_rect_of(&format!("LootButton{i}{part}"), (1024, 768)).is_some(), "LootButton{i}{part} is missing");
            }
        }
        for copper in [1, 50, 500, 5000, 50000, 500000] {
            assert!(mount.read(&format!("{}.blp", coin_icon(copper))).is_ok(), "coin icon for {copper}");
        }
        for path in crate::items::all().iter().map(|i| i.icon).chain(SLOT_BACKGROUNDS).chain([BACKPACK_PORTRAIT]) {
            assert!(mount.read(&format!("{path}.blp")).is_ok(), "{path} is missing");
        }
        // The sheet and the backpack start closed, and the character frame hangs off the top left.
        assert!(!tree.is_name_shown(CHARACTER) && !tree.is_name_shown(BACKPACK));
    }
}
