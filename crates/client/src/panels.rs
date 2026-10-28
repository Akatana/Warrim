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

mod questlog;
mod merchant;
mod gossip;
mod quest;
mod progress;
mod skills;
mod bag;
mod loot;
mod input;
mod tooltips;
mod draw;
mod magic;
mod keybinds;
mod levelup;
mod map;
mod menu;
mod sleep;
mod book;
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

/// What a spell asks of the game beyond what the panels hold; the game does it at the next frame (`Panels::take_requests`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    /// Take what is on the character of these kinds (bit `n` for kind `n`: 1 magic, 2 curse, 3 disease, 4 poison) off him.
    Cleanse(u32),
    /// The creatures that fight the character lose him.
    Vanish,
    /// Forward this many yards at once.
    Blink(f32),
    /// Leap at a place (render space).
    Leap(Vec3),
    /// A creature (an index into the content's creatures) is called to be fought before him (the demon of a binding).
    Foe(usize),
    /// The beast of this kind (an index) that he has selected is tamed for a while, and counts as killed for his quest.
    Tame(usize),
    /// The character's eyes leave him: the seconds, and how far (yards) they go.
    Scout { seconds: f32, reach: f32 },
    /// Take the shape of the beast of this display row, which adds this percent to the running speed (or give it back).
    Form { display: u32, percent: i32 },
    /// The pet learned a better rank of its spell: it casts it from now on.
    PetSpell,
}

pub struct Panels {
    /// Who the character is, for the texts that name it.
    name: String,
    race: crate::character::Race,
    sex: crate::character::Sex,
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
    /// Seconds left before each spell can be cast again.
    pub cooldowns: Vec<f32>,
    /// Seconds left of the global cooldown: no spell can begin until it is over (it starts when a cast begins, as in the original).
    pub gcd: f32,
    /// The spellbook's tab and the page each tab is at, the spell a click on it asked to cast, and the skills tab's list.
    book_tab: usize,
    book_page: [usize; magic::BOOK_TABS],
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
    /// The harmful and the helpful auras of the target, as the target frame lists them (for the tooltips of its buttons).
    target_auras: (Vec<crate::creature::Aura>, Vec<crate::creature::Aura>),
    calm: bool,
    /// Health and mana that were given at once and are for the player to take (`take_restores`).
    restored: (f32, f32),
    /// A mount item was used (`take_steed`): its display and speed bonus.
    steed: Option<(u32, i32)>,
    /// What spells asked of the game since the last frame.
    pub requests: Vec<Request>,
    /// The id of the creature that is the character's pet.
    pub pet: Option<String>,
    /// The spells of the original that the character taught his pet (the ranks of its firebolt, from grimoires).
    pub pet_spells: Vec<u32>,
    /// Seconds before the pact of the pet is renewed.
    pub pet_aura: f32,
    /// The type of creature the character tracks (shown on the world map), and where those are now (render space).
    tracking: Option<u32>,
    tracked: Vec<Vec3>,
    /// Whether the character rides (set by the game), and the mount being cast: seconds left, the steed's display and speed bonus.
    mounted: bool,
    mount_cast: Option<(f32, (u32, i32))>,
    /// A lure or a stone to put on the weapon: the item whose cast the game is to start, and whether the cast is over (then using it does it).
    lure: (Option<usize>, bool),
    /// Disenchant was chosen in the spellbook and waits for an item of the pack; the pack slot whose cast the game is to start.
    disenchant: (bool, Option<usize>),
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
    /// What is being read (`book.rs`).
    book: Option<book::Book>,
    /// The question that is asked: abandon this quest? (`StaticPopup1`)
    popup: Option<usize>,
    /// The hour being chosen to sleep until (the same popup asks it), and the one chosen, for the game to take (`take_sleep`).
    sleep: Option<f32>,
    slept: Option<f32>,
    sleep_dragging: bool,
    /// The question of the popup is whether to destroy the item the mouse carries (`DELETE_ITEM`).
    destroying: bool,
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
const SCROLL_FRAMES: [&str; 7] = ["ItemTextScrollFrame", "TalentFrameScrollFrame", "GossipGreetingScrollFrame", "QuestDetailScrollFrame", "QuestProgressScrollFrame", "QuestRewardScrollFrame", "QuestLogDetailScrollFrame"];
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

/// `$g male:female;` (the original's words that go by the sex of the one spoken to) as the word for `sex`.
fn gendered(text: &str, sex: crate::character::Sex) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("$g").or_else(|| rest.find("$G")) {
        let Some((words, after)) = rest[at + 2..].split_once(';') else { break };
        out.push_str(&rest[..at]);
        out.push_str(words.trim().split(':').nth((sex == crate::character::Sex::Female) as usize).unwrap_or(""));
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The text of the original with the player filled in, for the one named `name` of race `race`.
fn speak_to(name: &str, race: crate::character::Race, sex: crate::character::Sex, text: &str) -> String {
    let race = race.name();
    gendered(text, sex).replace("$N", name).replace("$n", name).replace("$R", race).replace("$r", &race.to_lowercase()).replace("$C", "Paladin").replace("$c", "paladin")
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
        book::setup(&mut tree)?;
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
        // The original leaves 205 for the minimap, which is not here: the icons go to the corner (`draw_sneak` follows).
        tree.set_point("BuffFrame", Point::TopRight, Some("UIParent"), Some(Point::TopRight), -10.0, -13.0);
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
        // No target of the target (the auras are listed by `set_target`).
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

        let mut panels = Panels { name: "Adventurer".into(), race: Default::default(), sex: Default::default(), tree, mount, stats: Stats::of(&inv.equipment, 1, &[0; 7], &[]), progress: Progress::new(), found: vec![false; content::get().pois.len()], map_open: false, map: Default::default(), explored: Vec::new(), travel: None, skills: crate::skills::Skills::new(), recipes: Default::default(), known: content::get().start.spells.clone(), learned: Vec::new(), cooldowns: vec![0.0; content::get().spells.len()], gcd: 0.0, book_tab: 0, book_page: [0; magic::BOOK_TABS], cast_bar: Default::default(), breath_shown: false, spell_click: None, drag: None, skill_list: Default::default(), talents: Default::default(), trade: Default::default(), trainer: Default::default(), craft_request: None, talent_tab: 0, inv, bag_scroll: 0, doll, portrait, doll_turn: 0.0, cursor: None, equipment_changed: false, stats_dirty: false, loot: None, closed_loot: None, events: Vec::new(), messages: Vec::new(), gossip: None, merchant: None, merchant_page: 0, consumed: Default::default(), afflicted: Vec::new(), target_auras: Default::default(), calm: true, restored: (0.0, 0.0), steed: None, requests: Vec::new(), pet: None, pet_spells: Vec::new(), pet_aura: 0.0, tracking: None, tracked: Vec::new(), mounted: false, mount_cast: None, lure: (None, false), disenchant: (false, None), buyback: None, quests: QuestLog::default(), quest_targets: Vec::new(), gossip_lines: Vec::new(), questing: None, seen: Default::default(), ready: Default::default(), log_selected: 0, reward_choice: None, level_ups: 0, book: None, popup: None, sleep: None, slept: None, sleep_dragging: false, destroying: false, points: 0, spent: [0; 3], level_open: false, level_press: None, level_art: None, target_portrait, target: None, menu: Default::default(), keys: Default::default() };
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
        self.popup.is_some() || self.sleep.is_some() || self.destroying
    }

    pub fn any_open(&self) -> bool {
        self.level_open || self.windows_open()
    }

    /// Whether a window of the original's interface is open (the level-up panel is not one: it gives way to them).
    fn windows_open(&self) -> bool {
        self.popup_open() || self.book_open() || self.character_open() || self.backpack_open() || self.loot_open() || self.gossip_open() || self.merchant_open() || self.quest_open() || self.quest_log_open() || self.spellbook_open() || self.talents_open() || self.trade_open() || self.trainer_open() || self.map_open || self.menu_open() || self.options_open()
    }

    // ---- the question

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
            self.close_book();
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
        self.close_book();
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
        let (x, y, size) = (screen.0 as f32 - 10.0 * s - 32.0 * s, 13.0 * s, 32.0 * s);
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
        let mut boosts = self.consumed.boosts();
        boosts.iter_mut().zip(self.spent_attributes()).for_each(|(b, a)| *b += a);
        self.stats = Stats::of_with(&self.inv.equipment, self.progress.level, &boosts, &timed, &self.talents.perks());
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
    #[test]
    fn words_that_go_by_sex_are_the_ones_of_the_character() {
        use crate::character::Sex;
        assert_eq!(super::gendered("Hail, $g lord:lady;. Well met, $g sir:madam;.", Sex::Male), "Hail, lord. Well met, sir.");
        assert_eq!(super::gendered("Hail, $g lord:lady;.", Sex::Female), "Hail, lady.");
        assert_eq!(super::gendered("No words.", Sex::Female), "No words.");
    }

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
