//! The game's own data, read at startup from the `content` folder (embedded in the program): items, what creatures
//! drop, the kinds of creatures and where they stand, and what the player starts with. The original keeps all of this
//! on the server. Things refer to each other by string id (`"wolf_meat"`); at load every reference is resolved and
//! checked, so a typo is an error at startup (and in the tests) and never a crash in the middle of the game.

use crate::character::Appearance;
use crate::items::{self, ItemDef, Look, Stack, Weapon, Wear, SLOTS};
use crate::loot::DropTable;
use anyhow::{bail, ensure, Context, Result};
use mpq::Mount;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::RwLock;
use wowfmt::dbc::Dbc;

/// What choosing a line of a creature's gossip does.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Action {
    /// The creature answers with this text.
    Say(String),
    /// The creature's wares are shown (`vendor` in its definition).
    Vendor,
    /// The creature teaches `skill` up to `rank`, for `cost` copper, to a character of at least `level` who already has rank `needs` of it.
    Train { skill: crate::skills::Skill, rank: u32, needs: u32, level: u32, cost: u32 },
}

/// A line the player can choose when talking to a creature.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GossipOption {
    pub text: String,
    pub action: Action,
}

/// What a creature says when talked to (`$N` is the player's name, `$R` race, `$C` class) and what the player may answer.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Gossip {
    pub greeting: String,
    #[serde(default)]
    pub options: Vec<GossipOption>,
}

/// What a creature looks like.
#[derive(Debug)]
pub enum Body {
    /// An animal or monster: one model with its skin textures, (creature skin slot, texture file) pairs, at a display scale.
    Beast { model: String, skins: Vec<(u32, String)>, scale: f32 },
    /// A person: the human male model dressed in these items (indexes into the items), like the player.
    Humanoid { outfit: Vec<usize>, look: Appearance },
}

/// One event of the script of a creature, as the original's EventAI has it (made by `tools/import_creature_ai.py`, played by `brain.rs`): the event type
/// `on` with its parameters `p`, the EventAI `flags`, the percent `chance`, the phases it is `off` in (a bit for each), and what it does: `acts`, each an action
/// type with its three parameters.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrainEvent {
    pub on: u8,
    pub p: Vec<i32>,
    pub flags: u32,
    pub chance: u32,
    pub off: u32,
    pub acts: Vec<(u8, i32, i32, i32)>,
}

impl Default for BrainEvent {
    fn default() -> BrainEvent {
        BrainEvent { on: 0, p: Vec::new(), flags: 0, chance: 100, off: 0, acts: Vec::new() }
    }
}

impl BrainEvent {
    /// Parameter `n` of the event (0 when it has none).
    pub fn param(&self, n: usize) -> i32 {
        self.p.get(n).copied().unwrap_or(0)
    }
}

/// How a creature fights (`content/classic_creature_ai.ron`): its mana, the seconds between its swings (0 for the usual two), the damage of its ranged
/// weapon (what Shoot and Throw do), the yards within which it calls others of its faction to help when it starts a fight, whether it only fights back,
/// its faction, and its script.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Brain {
    pub mana: f32,
    pub swing: f32,
    pub shot: f32,
    pub assist: f32,
    pub calm: bool,
    pub faction: u32,
    pub events: Vec<BrainEvent>,
}

impl Default for Brain {
    fn default() -> Brain {
        Brain { mana: 0.0, swing: 0.0, shot: 0.0, assist: 10.0, calm: false, faction: 0, events: Vec::new() }
    }
}

impl Brain {
    /// Whether it is the brain of a creature that only bites (what a file need not say).
    pub fn is_default(&self) -> bool {
        *self == Brain::default()
    }
}

/// A kind of creature.
#[derive(Debug)]
pub struct CreatureDef {
    pub id: String,
    pub name: String,
    /// Its level: what it is worth in experience, and how it compares with the player's.
    pub level: u32,
    /// The line under the name: `<Stormwind Guard>` (for a tooltip, which the game does not have yet).
    #[allow(dead_code)]
    pub title: Option<String>,
    pub body: Body,
    /// Attacks the player when he comes near.
    pub hostile: bool,
    /// Leaves the player alone (yellow name) until he attacks it, then fights back; neither hostile nor neutral is a friend to talk to.
    pub neutral: bool,
    pub health: f32,
    pub bite_damage: f32,
    /// Seconds from death until it is back.
    pub respawn: f32,
    /// Id of its drop table, if it drops anything.
    pub loot: Option<String>,
    /// What it says when talked to.
    pub gossip: Option<Gossip>,
    /// What it sells (indexes into the items), in the order of the window.
    pub vendor: Vec<usize>,
    /// How it fights beyond the bite: what it casts and shoots, with what mana, and who it calls.
    pub brain: Brain,
}

impl CreatureDef {
    /// Whether the player can fight it: a hostile or a neutral creature.
    pub fn attackable(&self) -> bool {
        self.hostile || self.neutral
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub enum RawBody {
    Beast { model: String, skins: Vec<(u32, String)>, scale: f32 },
    Humanoid {
        outfit: Vec<String>,
        #[serde(default)]
        look: Appearance,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawCreature {
    pub id: String,
    pub name: String,
    #[serde(default = "one")]
    pub level: u32,
    #[serde(default)]
    pub title: Option<String>,
    pub body: RawBody,
    #[serde(default)]
    pub hostile: bool,
    #[serde(default)]
    pub neutral: bool,
    #[serde(default = "hundred")]
    pub health: f32,
    #[serde(default)]
    pub bite_damage: f32,
    #[serde(default = "respawn_time")]
    pub respawn: f32,
    #[serde(default)]
    pub loot: Option<String>,
    #[serde(default)]
    pub gossip: Option<Gossip>,
    #[serde(default)]
    pub vendor: Vec<String>,
    /// How it fights beyond the bite (its script): from `classic_creature_ai.ron` for the original's creatures, edited in the editor.
    #[serde(default, skip_serializing_if = "Brain::is_default")]
    pub brain: Brain,
}

fn hundred() -> f32 {
    100.0
}
fn respawn_time() -> f32 {
    300.0
}

/// A creature standing somewhere: its kind, the place in WoW coordinates (north, west), and the heading in degrees.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub creature: String,
    pub at: (f32, f32),
    pub facing: f32,
    /// How far from `at` it strolls, yards (0 stands still): the database's `spawndist`, whose usual value is the default.
    #[serde(default = "default_wander", skip_serializing_if = "is_default_wander")]
    pub wander: f32,
}

/// The database's default `spawndist`.
pub const DEFAULT_WANDER: f32 = 5.0;

fn default_wander() -> f32 {
    DEFAULT_WANDER
}

fn is_default_wander(w: &f32) -> bool {
    *w == DEFAULT_WANDER
}

/// A model put into the world by the editor: a doodad (`.m2`) or a building (`.wmo`), as the map files place them. The place is in WoW
/// coordinates (north, west, height) and the turn is in degrees in the order of the map files (tilt, heading, roll).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prop {
    pub model: String,
    /// The map it stands in ("Azeroth", "Kalimdor"); none for what the editor puts down, which belongs to the map being played.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<String>,
    /// The zone it stands in, for finding it in the editor (the original's objects have it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// What picking it up gives (a quest item): it is drawn apart from the map, can only be used while a quest still wants the item, and is gone for a while after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pickup: Option<Pickup>,
    /// A bed: using it asks until what hour to sleep, and the sleeper wakes fully healed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sleep: bool,
    pub at: (f32, f32, f32),
    pub rotation: [f32; 3],
    pub scale: f32,
}

/// What a prop gives when it is picked up: the item (an id), whether the prop is gone after it (`vanish`: a sack, a flower; one that is not stays and
/// can be used again for as long as a quest wants the item) and the seconds until a prop that went is there again.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pickup {
    pub item: String,
    #[serde(default = "Pickup::respawn_default")]
    pub respawn: f32,
    #[serde(default = "Pickup::vanish_default")]
    pub vanish: bool,
}

impl Pickup {
    pub fn respawn_default() -> f32 {
        120.0
    }

    pub fn vanish_default() -> bool {
        true
    }
}

/// What the player begins with: worn items by equipment slot, and the backpack from its first slot on.
pub struct Start {
    /// The spells it knows (indices).
    pub spells: Vec<usize>,
    pub money: u32,
    pub equipment: Vec<(usize, Stack)>,
    pub bag: Vec<Stack>,
}

/// Something a quest asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    /// Kill this many of a kind of creature (an index into the creatures).
    Kill { creature: usize, count: u32 },
    /// Carry this many of an item (an index into the items) to the turn in.
    Collect { item: usize, count: u32 },
}

impl Quest {
    /// The rewards as the window lists them: the items to choose from first, then those everyone gets.
    pub fn rewards(&self) -> impl Iterator<Item = &Stack> {
        self.choice.iter().chain(&self.items)
    }
}

/// A quest (`content/quests.ron`).
#[derive(Debug)]
pub struct Quest {
    pub id: String,
    pub title: String,
    /// The creatures (indexes) that offer it and take it back.
    pub giver: usize,
    pub turn_in: usize,
    pub details: String,
    pub objective: String,
    pub progress: String,
    pub completion: String,
    pub goals: Vec<Goal>,
    pub copper: u32,
    /// Experience for the turn in.
    pub xp: u32,
    /// Items the player gets, and items of which he takes one (when there are any).
    pub items: Vec<Stack>,
    pub choice: Vec<Stack>,
    /// The quest (an earlier index) that has to be done first.
    pub requires: Option<usize>,
}

/// A chest in the world (`content/chests.ron`).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChestDef {
    pub id: String,
    pub model: String,
    /// The place in WoW coordinates (north, west) and the heading in degrees.
    pub at: (f32, f32),
    pub facing: f32,
    /// How hard the lock is; none for a chest that is not locked.
    #[serde(default)]
    pub lock: Option<crate::lockpick::Level>,
    /// Id of its drop table (what it holds by chance); none for a chest that holds only what is fixed.
    #[serde(default)]
    pub loot: String,
    /// What it always holds, besides what the table gives: copper, and items (id, how many).
    #[serde(default)]
    pub copper: u32,
    #[serde(default)]
    pub items: Vec<(String, u32)>,
}

/// A place the compass can show and the player can discover.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Poi {
    pub id: String,
    pub name: String,
    /// The place in WoW coordinates (north, west), and how close the player must come to discover it, yards.
    pub at: (f32, f32),
    pub radius: f32,
    /// Its picture: a cell of `Interface\Minimap\POIIcons` (8 by 8, counted along the rows); the default is a building.
    #[serde(default = "building")]
    pub icon: usize,
    /// Feet height there, from the original's nearest spawn (a floor of a building, a bridge); none: the terrain.
    #[serde(default)]
    pub height: Option<f32>,
}

fn building() -> usize {
    4
}

/// What a spell costs to cast.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Cost {
    Mana(f32),
    Stamina(f32),
}

/// What a spell does. Damage and heals are rolled between `min` and `max` and grow with the rank of the spell's class skill.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Effect {
    /// Hurts the selected creature if it is within `range` yards.
    Damage { min: f32, max: f32, range: f32 },
    /// Hurts every hostile creature within `radius` yards of the caster.
    Area { min: f32, max: f32, radius: f32 },
    /// Heals the caster.
    Heal { min: f32, max: f32 },
}

/// A spell of a class (`content/spells.ron`). `level` is the rank of the class skill that its tome asks for to be learned: 1, the
/// basic spells, asks for nothing.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellDef {
    pub id: String,
    /// Its row of the original's `Spell.dbc`: cast time, pictures and sounds come from there (`visuals.rs`).
    pub spell: u32,
    pub name: String,
    pub class: crate::skills::Skill,
    pub level: u32,
    /// The icon's name in `Interface\Icons`.
    pub icon: String,
    pub cost: Cost,
    pub cooldown: f32,
    pub effect: Effect,
    pub description: String,
}

/// One of the (up to three) effects of a spell of the original (`content/classic_spells.ron`): `kind` is its Effect, `aura` the aura it applies,
/// `low`..`high` its points, `period` the milliseconds between ticks, `targets` the implicit target types.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClassicEffect {
    pub kind: u32,
    pub aura: u32,
    pub low: i32,
    pub high: i32,
    pub per_level: f32,
    pub radius: f32,
    pub period: u32,
    pub misc: i32,
    pub trigger: u32,
    pub item: u32,
    pub chain: u32,
    pub targets: (u32, u32),
}

/// A spell of the original game, as its tables have it (`content/classic_spells.ron`, made by `tools/import_spells.py`). Not a spell the
/// player can cast: those are `SpellDef`s. `spell` is the row of `Spell.dbc` (pictures and sounds); times are milliseconds, distances yards.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClassicSpell {
    pub id: String,
    pub spell: u32,
    pub name: String,
    pub rank: String,
    /// The icon's name in `Interface\Icons`.
    pub icon: String,
    pub school: u32,
    pub mechanic: u32,
    pub dispel: u32,
    pub level: u32,
    pub base_level: u32,
    pub max_level: u32,
    /// The power it costs (0 mana, 1 rage, 3 energy) and how much of it, or what percent of the base.
    pub power: u32,
    pub cost: u32,
    pub cost_percent: u32,
    pub cast: u32,
    pub cooldown: u32,
    pub range: f32,
    pub duration: u32,
    pub attributes: u32,
    pub family: u32,
    pub family_flags: u64,
    pub stack: u32,
    /// Proc flags, chance and charges.
    pub proc: (u32, u32, u32),
    /// Items (entries) and how many of them it uses up.
    pub reagents: Vec<(u32, u32)>,
    pub effects: Vec<ClassicEffect>,
    pub text: String,
}

impl ClassicEffect {
    /// Whether the effect is for the enemies of the caster (by its implicit targets, MaNGOS `Targets`: a chain or an area of enemies, the front of the caster).
    pub fn enemy(&self) -> bool {
        [self.targets.0, self.targets.1].iter().any(|t| matches!(t, 2 | 6 | 15 | 16 | 24 | 28 | 53 | 54))
    }

    /// Whether the effect is for the caster and its friends (itself, its party, friendly units around it).
    pub fn friendly(&self) -> bool {
        !self.enemy() && matches!(self.targets.0, 0 | 1 | 3 | 5 | 20 | 21 | 27 | 30 | 31 | 33 | 34 | 35 | 37 | 56 | 57 | 61)
    }

    /// Whether the effect hits an area (of enemies or friends) instead of one unit, and whether that area is a cone in front of the caster.
    pub fn area(&self) -> bool {
        [self.targets.0, self.targets.1].iter().any(|t| matches!(t, 15 | 16 | 20 | 24 | 28 | 30 | 31 | 54 | 56))
    }

    pub fn cone(&self) -> bool {
        [self.targets.0, self.targets.1].iter().any(|t| matches!(t, 24 | 54))
    }
}

impl ClassicSpell {
    /// The points of effect `n` for a caster of `level` as MaNGOS works them out: a number from `low` to `high` picked with `roll`, and what each level above
    /// the spell's own adds (up to its highest level, if it has one).
    pub fn points(&self, n: usize, level: u32, roll: u32) -> f32 {
        let e = &self.effects[n];
        let base = e.low + (roll % ((e.high - e.low).max(0) as u32 + 1)) as i32;
        let level = if self.max_level > 0 { level.min(self.max_level) } else { level };
        base as f32 + e.per_level * level.saturating_sub(self.base_level) as f32
    }

    /// Whether it is a ranged attack (Shoot, Throw): what they do is the damage of the creature's ranged weapon.
    pub fn is_ranged_attack(&self) -> bool {
        self.attributes & 0x2 != 0
    }

    /// Mana it takes a creature with `base` mana at most.
    pub fn mana_cost(&self, base: f32) -> f32 {
        if self.power != 0 {
            0.0
        } else if self.cost_percent > 0 {
            base * self.cost_percent as f32 / 100.0
        } else {
            self.cost as f32
        }
    }
}

pub struct Content {
    pub items: Vec<ItemDef>,
    pub spells: Vec<SpellDef>,
    pub classic_spells: Vec<ClassicSpell>,
    /// The place in `classic_spells` of each spell (its row in `Spell.dbc`).
    pub classic_index: std::collections::HashMap<u32, usize>,
    pub pois: Vec<Poi>,
    pub chests: Vec<ChestDef>,
    pub quests: Vec<Quest>,
    pub loot: BTreeMap<String, DropTable>,
    pub creatures: Vec<CreatureDef>,
    pub spawns: Vec<Spawn>,
    pub props: Vec<Prop>,
    pub start: Start,
}

/// The content in use. It can be replaced while the game runs (`reload`); what it replaces is never freed, because references into it
/// (`&'static`) are held by whatever was made from it.
static CONTENT: RwLock<Option<&'static Content>> = RwLock::new(None);

fn current() -> Option<&'static Content> {
    *CONTENT.read().unwrap_or_else(|e| e.into_inner())
}

fn install(content: Content) {
    *CONTENT.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::leak(Box::new(content)));
}

/// Read the content; `mount` gives the items their icons and looks from `ItemDisplayInfo.dbc`. Call once at startup.
pub fn init(mount: &Mount) -> Result<()> {
    let content = load(Some(mount))?;
    anyhow::ensure!(current().is_none(), "content loaded twice");
    install(content);
    Ok(())
}

/// Read the content files again and use them from now on. Refused (and the content stays) when they are not valid or when the spells
/// differ: the action bar, the spellbook and the casting hold spells by their numbers and the spells are not edited live.
pub fn reload(mount: &Mount) -> Result<()> {
    let new = load(Some(mount))?;
    let old = get();
    anyhow::ensure!(old.spells.len() == new.spells.len() && old.spells.iter().zip(&new.spells).all(|(a, b)| a.id == b.id && a.spell == b.spell), "the spells changed");
    install(new);
    Ok(())
}

/// The content. In the tests it loads itself, with the game data if it is installed.
pub fn get() -> &'static Content {
    current().unwrap_or_else(first)
}

#[cfg(test)]
fn first() -> &'static Content {
    static LOADING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _loading = LOADING.lock().unwrap_or_else(|e| e.into_inner());
    if current().is_none() {
        let mount = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok());
        install(load(mount.as_ref()).expect("the content files are valid"));
    }
    current().expect("installed above")
}

#[cfg(not(test))]
fn first() -> &'static Content {
    panic!("content::init was not called")
}

pub fn creature(id: &str) -> &'static CreatureDef {
    &get().creatures[creature_index(id)]
}

/// The index of the creature kind with this id.
pub fn creature_index(id: &str) -> usize {
    get().creatures.iter().position(|c| c.id == id).unwrap_or_else(|| panic!("no creature {id}"))
}

/// The spell of the original with this row of `Spell.dbc`.
pub fn classic_spell(id: u32) -> Option<&'static ClassicSpell> {
    let c = get();
    c.classic_index.get(&id).map(|&i| &c.classic_spells[i])
}

pub fn spell(index: usize) -> &'static SpellDef {
    &get().spells[index]
}

pub fn spell_index(id: &str) -> usize {
    get().spells.iter().position(|s| s.id == id).unwrap_or_else(|| panic!("no spell {id}"))
}

pub fn quest(index: usize) -> &'static Quest {
    &get().quests[index]
}

pub fn quest_index(id: &str) -> usize {
    get().quests.iter().position(|q| q.id == id).unwrap_or_else(|| panic!("no quest {id}"))
}

pub fn drop_table(id: &str) -> Option<&'static DropTable> {
    get().loot.get(id)
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawItem {
    pub id: String,
    pub name: String,
    /// Row of `ItemDisplayInfo.dbc`.
    pub display: u32,
    #[serde(default = "common")]
    pub quality: usize,
    #[serde(default = "no_wear")]
    pub wear: Wear,
    #[serde(default)]
    pub kind: (String, String),
    #[serde(default)]
    pub armor: i32,
    #[serde(default)]
    pub block: i32,
    #[serde(default)]
    pub stats: (i32, i32, i32, i32, i32),
    #[serde(default)]
    pub weapon: Option<Weapon>,
    #[serde(default = "one")]
    pub stack: u32,
    #[serde(default = "cloth")]
    pub handling: String,
    /// The id of the spell that reading this teaches.
    #[serde(default)]
    pub tome: Option<String>,
    /// A quest item: it drops only while a quest being worked on wants it, and is not sold.
    #[serde(default)]
    pub quest: bool,
    #[serde(default)]
    pub buy: u32,
    #[serde(default)]
    pub sell: u32,
    /// Made from the original's item data (`content/classic_items.ron`): what its tables do not have (a look) is shown as in the original.
    #[serde(default)]
    pub classic: bool,
    /// The class and subclass of the original's item tables (0 consumable, 1 container, 2 weapon, 4 armor, 6 projectile, 7 trade goods, 9 recipe,
    /// 11 quiver, 12 quest, 13 key, 15 miscellaneous).
    #[serde(default)]
    pub class: u8,
    #[serde(default)]
    pub sub: u8,
    /// The level needed to use it, and the classes and races it is for (bit masks of the original; 0 for all).
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub classes: u32,
    #[serde(default)]
    pub races: u32,
    #[serde(default)]
    pub bind: items::Bind,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub durability: u32,
    #[serde(default)]
    pub health: i32,
    #[serde(default)]
    pub mana: i32,
    /// Holy, fire, nature, frost, shadow and arcane resistance.
    #[serde(default)]
    pub resist: [i32; 6],
    /// More damage of a weapon: lowest, highest, school.
    #[serde(default)]
    pub more: Vec<(f32, f32, u8)>,
    #[serde(default = "one")]
    pub buy_count: u32,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub spells: Vec<items::ItemSpell>,
    #[serde(default)]
    pub skill: (u32, u32),
    #[serde(default)]
    pub slots: u32,
    #[serde(default)]
    pub flags: u32,
    /// What one of it weighs; without one, `default_weight` of its kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// What one of an item weighs when its entry gives no weight: by where it is worn and what armor it is, else by its class.
fn default_weight(raw: &RawItem) -> f32 {
    match (raw.wear, raw.class, raw.sub) {
        (Wear::TwoHand, ..) => 6.0,
        (Wear::Shield, ..) => 5.0,
        (Wear::OneHand | Wear::MainHand | Wear::OffHand, ..) => 3.0,
        (Wear::Ranged, ..) => 2.0,
        (Wear::Held, ..) => 0.5,
        (Wear::Back, ..) => 1.0,
        (Wear::Thrown, ..) => 0.1,
        (Wear::Ammo, ..) => 0.02,
        (Wear::Neck | Wear::Finger | Wear::Trinket | Wear::Shirt | Wear::Tabard, ..) => 0.2,
        (Wear::None, class, _) => match class {
            1 | 11 => 1.0,
            3 | 5 | 7 | 12 => 0.1,
            6 => 0.02,
            13 => 0.05,
            0 | 9 => 0.2,
            _ => 0.5,
        },
        (_, _, sub) => match sub {
            1 => 1.0,
            3 => 4.0,
            4 => 8.0,
            _ => 2.0,
        },
    }
}

fn common() -> usize {
    1
}
fn no_wear() -> Wear {
    Wear::None
}
fn one() -> u32 {
    1
}
fn cloth() -> String {
    "Cloth_Leather".into()
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum RawGoal {
    Kill(String, u32),
    Collect(String, u32),
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawReward {
    #[serde(default)]
    pub copper: u32,
    #[serde(default)]
    pub xp: u32,
    #[serde(default)]
    pub items: Vec<(String, u32)>,
    #[serde(default)]
    pub choice: Vec<(String, u32)>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawQuest {
    pub id: String,
    pub title: String,
    pub giver: String,
    pub turn_in: String,
    pub details: String,
    pub objective: String,
    pub progress: String,
    pub completion: String,
    #[serde(default)]
    pub goals: Vec<RawGoal>,
    #[serde(default)]
    pub requires: Option<String>,
    #[serde(default)]
    pub reward: RawReward,
}

/// What a creature or chest drops (`content/loot.ron`): copper from low to high, and items with a count (low, high) and a chance in percent.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawTable {
    pub copper: (u32, u32),
    pub items: Vec<(String, (u32, u32), f32)>,
    /// Other tables that are rolled as well (their copper is added, and their items are among these): the original's tables of `classic_loot.ron`.
    #[serde(default)]
    pub also: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStart {
    #[serde(default)]
    spells: Vec<String>,
    money: u32,
    equipment: Vec<(usize, String)>,
    bag: Vec<(String, u32)>,
}

/// Text that lives as long as the program: the content is read once, and `ItemDef` stays a small `Copy` type. The same text is kept once, so that
/// reading the content again (the editor's Apply) does not keep another copy of every name.
fn leak(s: impl Into<String>) -> &'static str {
    static KEPT: std::sync::Mutex<Option<HashSet<&'static str>>> = std::sync::Mutex::new(None);
    let s = s.into();
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let kept = kept.get_or_insert_with(HashSet::new);
    if let Some(&text) = kept.get(s.as_str()) {
        return text;
    }
    let text: &'static str = Box::leak(s.into_boxed_str());
    kept.insert(text);
    text
}

/// `ItemDisplayInfo.dbc`, with its rows found by their numbers.
struct Displays<'a> {
    dbc: &'a Dbc,
    rows: std::collections::HashMap<u32, usize>,
    /// The icons the game files have (lower case names without folder and ending): an item whose icon is missing shows the question mark.
    icons: HashSet<String>,
}

impl<'a> Displays<'a> {
    fn new(dbc: &'a Dbc, mount: &Mount) -> Displays<'a> {
        let icons = mount.list().iter().filter_map(|f| f.to_ascii_lowercase().strip_prefix(r"interface\icons\").and_then(|n| n.strip_suffix(".blp")).map(String::from)).collect();
        Displays { rows: dbc.iter().enumerate().map(|(i, r)| (r.u32(0), i)).collect(), dbc, icons }
    }

    fn has_icon(&self, name: &str) -> bool {
        self.icons.contains(&name.to_ascii_lowercase())
    }

    fn row(&self, id: u32) -> Option<wowfmt::dbc::Record<'a>> {
        self.dbc.record(*self.rows.get(&id)?)
    }
}

/// The names of the kinds of weapons and armor of the original (`ItemSubClass.dbc`).
fn subclass_name(class: u8, sub: u8) -> &'static str {
    match (class, sub) {
        (2, 0) => "Axe",
        (2, 1) => "Axe",
        (2, 2) => "Bow",
        (2, 3) => "Gun",
        (2, 4) => "Mace",
        (2, 5) => "Mace",
        (2, 6) => "Polearm",
        (2, 7) => "Sword",
        (2, 8) => "Sword",
        (2, 10) => "Staff",
        (2, 13) => "Fist Weapon",
        (2, 14) => "Miscellaneous",
        (2, 15) => "Dagger",
        (2, 16) => "Thrown",
        (2, 17) => "Spear",
        (2, 18) => "Crossbow",
        (2, 19) => "Wand",
        (2, 20) => "Fishing Pole",
        (4, 1) => "Cloth",
        (4, 2) => "Leather",
        (4, 3) => "Mail",
        (4, 4) => "Plate",
        (4, 6) => "Shield",
        (4, 7) => "Libram",
        (4, 8) => "Idol",
        (4, 9) => "Totem",
        (6, 2) => "Arrow",
        (6, 3) => "Bullet",
        _ => "",
    }
}

/// The type line of the tooltip of an item that has none of its own: where it is worn, and what kind of armor or weapon it is.
fn derived_kind(raw: &RawItem) -> (String, String) {
    if raw.wear == Wear::None {
        return (String::new(), String::new());
    }
    let sub = subclass_name(raw.class, raw.sub);
    let place = if raw.class == 4 && matches!(raw.sub, 7..=9) { "Relic" } else { raw.wear.name() };
    (place.to_string(), sub.to_string())
}

/// What a row of `ItemDisplayInfo.dbc` makes of the look of an item worn as `wear`: the textures painted on the skin (fields 14-21), the model held in the
/// hand (1 and 3; none for what has no model, such as a wand or a relic), and the rest (mesh parts 7-9, models 1-2, their textures 3-4, helmet rows 12-13).
pub fn look_of(row: &wowfmt::dbc::Record, wear: Wear) -> (Look, Option<(&'static str, &'static str)>, items::Visual) {
    let mut look = Look::NONE;
    for (i, part) in look.parts.iter_mut().enumerate() {
        let texture = row.string(14 + i);
        *part = (!texture.is_empty()).then(|| leak(texture));
    }
    let mut held = None;
    if let (Some(folder), model) = (wear.holds(), row.string(1)) {
        let model = model.rsplit_once('.').map_or(model, |(stem, _)| stem);
        if !model.is_empty() && !row.string(3).is_empty() {
            held = Some((leak(format!(r"Item\ObjectComponents\{folder}\{model}.m2")), leak(format!(r"Item\ObjectComponents\{folder}\{}.blp", row.string(3)))));
        }
    }
    let name = |field: usize| Some(row.string(field)).filter(|n| !n.is_empty()).map(|n| leak(n.rsplit_once('.').map_or(n, |(stem, _)| stem)));
    let visual = items::Visual { geo: [row.u32(7) as u8, row.u32(8) as u8, row.u32(9) as u8], models: [name(1), name(2)], textures: [name(3), name(4)], helm: [row.u32(12) as u16, row.u32(13) as u16], enchant: row.u32(22) as u16, shot: row.u32(10) as u16 };
    (look, held, visual)
}

/// The icon, the skin textures and the held model of an item, from its row of `ItemDisplayInfo.dbc`
/// (fields: 1 model, 3 model texture, 5 icon, 14-21 the eight skin textures).
fn build_item(raw: RawItem, display: Option<&Displays>, spells: &[SpellDef]) -> Result<ItemDef> {
    let (mut icon, mut look, mut held, mut visual) = ("", Look::NONE, None, items::Visual::default());
    if let Some(displays) = display {
        match displays.row(raw.display) {
            Some(row) => {
                icon = if row.string(5).is_empty() || (raw.classic && !displays.has_icon(row.string(5))) { r"Interface\Icons\INV_Misc_QuestionMark" } else { leak(format!(r"Interface\Icons\{}", row.string(5))) };
                (look, held, visual) = look_of(&row, raw.wear);
            }
            // An item whose look the tables do not have shows the question mark, as in the original.
            None if raw.classic => icon = r"Interface\Icons\INV_Misc_QuestionMark",
            None => bail!("no ItemDisplayInfo row {}", raw.display),
        }
    }
    ensure!(raw.quality < items::QUALITY_COLORS.len(), "quality {}", raw.quality);
    ensure!(raw.stack >= 1, "stack must be at least 1");
    let kind = if raw.kind == Default::default() { derived_kind(&raw) } else { raw.kind.clone() };
    let (kind, s) = ((leak(kind.0), leak(kind.1)), raw.stats);
    let tome = raw.tome.as_deref().map(|id| spells.iter().position(|s| s.id == id).with_context(|| format!("tome of unknown spell {id}"))).transpose()?;
    ensure!(!raw.quest || raw.sell == 0, "{} is a quest item, which is not sold", raw.id);
    let item_spells: Vec<items::ItemSpellDef> = raw.spells.iter().map(|s| items::ItemSpellDef { spell: s.spell, trigger: s.trigger, charges: s.charges, cooldown: s.cooldown, effects: Box::leak(s.effects.clone().into_boxed_slice()), text: leak(s.text.clone()) }).collect();
    let weight = raw.weight.unwrap_or_else(|| default_weight(&raw));
    ensure!(weight >= 0.0, "{} weighs less than nothing", raw.id);
    Ok(ItemDef {
        weight,
        tome,
        quest: raw.quest,
        id: leak(raw.id),
        name: leak(raw.name),
        icon,
        quality: raw.quality,
        wear: raw.wear,
        kind,
        armor: raw.armor,
        block: raw.block,
        stats: [s.0, s.1, s.2, s.3, s.4],
        weapon: raw.weapon,
        look,
        held,
        stack: raw.stack,
        buy: raw.buy,
        sell: raw.sell,
        handling: leak(raw.handling),
        class: raw.class,
        subclass: raw.sub,
        level: raw.level,
        classes: raw.classes,
        races: raw.races,
        bind: raw.bind,
        unique: raw.unique,
        durability: raw.durability,
        health: raw.health,
        mana: raw.mana,
        resist: raw.resist,
        more_damage: Box::leak(raw.more.into_boxed_slice()),
        buy_count: raw.buy_count,
        text: leak(raw.text),
        spells: Box::leak(item_spells.into_boxed_slice()),
        skill: raw.skill,
        slots: raw.slots,
        flags: raw.flags,
        visual,
    })
}

/// The folder the content files are read from while the game runs, so that they can be changed (by hand, or by the editor) without
/// building the game again: `RSWORLD_CONTENT`, or the `content` folder of the source tree the game was built from. Without one the
/// files built into the program are used.
pub fn content_dir() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("RSWORLD_CONTENT").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"));
    dir.is_dir().then_some(dir)
}

/// The text of the content file `name`: from the folder if there is one (not while testing), else the one built in.
fn source(name: &str, embedded: &'static str) -> String {
    if cfg!(test) {
        return embedded.to_string();
    }
    content_dir().and_then(|d| std::fs::read_to_string(d.join(name)).ok()).unwrap_or_else(|| embedded.to_string())
}

/// The first lines of `props.ron` as the editor writes it.
const PROPS_HEADER: &str = "// Models put into the world by the editor (F1, Places, Props): the file of a model (World\\...\\*.m2, or a building *.wmo), the place in\n// WoW coordinates (north, west, height), the turn as the map files have it (degrees as (tilt, heading, roll)) and the scale.\n";

/// Write the props to `props.ron` of the content folder (to a file beside it first, so that a crash never leaves half of it).
pub fn write_props(props: &[Prop]) -> Result<()> {
    let dir = content_dir().context("there is no content folder to write to")?;
    let text = ron::ser::to_string_pretty(props, ron::ser::PrettyConfig::default())?;
    let (path, temp) = (dir.join("props.ron"), dir.join("props.ron.tmp"));
    std::fs::write(&temp, format!("{PROPS_HEADER}{text}\n"))?;
    std::fs::rename(&temp, &path)?;
    Ok(())
}

/// What the editor adds to a list of the content files: the entries it changed or made (an entry with the id of one of the file
/// replaces it) and the ids it removed.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Part<T> {
    pub removed: Vec<String>,
    pub entries: Vec<T>,
}

impl<T> Default for Part<T> {
    fn default() -> Part<T> {
        Part { removed: Vec::new(), entries: Vec::new() }
    }
}

/// An entry of a list that other things refer to by its id.
pub trait Keyed {
    fn key(&self) -> &str;
}

impl Keyed for RawItem {
    fn key(&self) -> &str {
        &self.id
    }
}
impl Keyed for RawCreature {
    fn key(&self) -> &str {
        &self.id
    }
}
impl Keyed for RawQuest {
    fn key(&self) -> &str {
        &self.id
    }
}
impl Keyed for Poi {
    fn key(&self) -> &str {
        &self.id
    }
}
impl Keyed for ChestDef {
    fn key(&self) -> &str {
        &self.id
    }
}

impl<T: Keyed + Clone + PartialEq> Part<T> {
    /// The list of the file with this part put on it.
    fn apply(&self, list: &mut Vec<T>) {
        list.retain(|e| !self.removed.iter().any(|r| r == e.key()));
        for entry in &self.entries {
            match list.iter().position(|e| e.key() == entry.key()) {
                Some(i) => list[i] = entry.clone(),
                None => list.push(entry.clone()),
            }
        }
    }

    /// What has to be put on `base` to make `current`.
    fn between(base: &[T], current: &[T]) -> Part<T> {
        Part {
            removed: base.iter().filter(|b| !current.iter().any(|c| c.key() == b.key())).map(|b| b.key().to_string()).collect(),
            entries: current.iter().filter(|c| base.iter().find(|b| b.key() == c.key()) != Some(c)).cloned().collect(),
        }
    }
}

/// The overlay file `editor.ron` of the content folder: what the editor changed in the lists of the content files. The spawns have no
/// ids: those it took away are named by their whole entry.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overlay {
    pub items: Part<RawItem>,
    pub creatures: Part<RawCreature>,
    pub quests: Part<RawQuest>,
    pub chests: Part<ChestDef>,
    pub pois: Part<Poi>,
    /// The loot tables by id: those it changed or made, and those it removed.
    pub loot_removed: Vec<String>,
    pub loot: BTreeMap<String, RawTable>,
    pub spawns_removed: Vec<Spawn>,
    pub spawns_added: Vec<Spawn>,
    pub props_removed: Vec<Prop>,
    pub props_added: Vec<Prop>,
}

impl Overlay {
    pub fn read() -> Result<Overlay> {
        let Some(path) = content_dir().map(|d| d.join("editor.ron")).filter(|p| p.exists()) else { return Ok(Overlay::default()) };
        ron::from_str(&std::fs::read_to_string(&path)?).with_context(|| format!("{} is not a content overlay", path.display()))
    }

    /// Write it to the content folder, to a file beside it first, so that a crash never leaves half of it.
    pub fn write(&self) -> Result<()> {
        let dir = content_dir().context("there is no content folder to write to")?;
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        let (path, temp) = (dir.join("editor.ron"), dir.join("editor.ron.tmp"));
        std::fs::write(&temp, format!("// Written by the in-game editor (F1): what it changed in the other files of this folder. Delete it to undo it all.\n{text}"))?;
        std::fs::rename(&temp, &path)?;
        Ok(())
    }

    /// What it takes to turn the lists of the content files (`base`) into `current`.
    pub fn between(base: &Raws, current: &Raws) -> Overlay {
        let (spawns_removed, spawns_added) = difference(&base.spawns, &current.spawns);
        let (props_removed, props_added) = difference(&base.props, &current.props);
        Overlay {
            items: Part::between(&base.items, &current.items),
            creatures: Part::between(&base.creatures, &current.creatures),
            quests: Part::between(&base.quests, &current.quests),
            chests: Part::between(&base.chests, &current.chests),
            pois: Part::between(&base.pois, &current.pois),
            loot_removed: base.loot.keys().filter(|k| !current.loot.contains_key(*k)).cloned().collect(),
            loot: current.loot.iter().filter(|(k, v)| base.loot.get(*k) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect(),
            spawns_removed,
            spawns_added,
            props_removed,
            props_added,
        }
    }
}

/// The objects the original game puts in its maps (`content/classic_props.ron`, made by `tools/import_props.py`): they are among the props, and the editor
/// keeps only what it changed of them (those it took away go to the overlay, those it moved or made are in `props.ron`).
pub fn classic_props() -> Result<Vec<Prop>> {
    ron::from_str(&source("classic_props.ron", include_str!("../../../content/classic_props.ron"))).context("classic_props.ron")
}

/// The entries of `base` that `current` lacks, and those of `current` that `base` lacks (a list of entries without ids: whole entries are compared).
pub fn difference<T: Clone + PartialEq>(base: &[T], current: &[T]) -> (Vec<T>, Vec<T>) {
    let mut unused: Vec<&T> = current.iter().collect();
    let mut removed = Vec::new();
    for b in base {
        match unused.iter().position(|c| *c == b) {
            Some(i) => {
                unused.remove(i);
            }
            None => removed.push(b.clone()),
        }
    }
    (removed, unused.into_iter().cloned().collect())
}

/// `list` without the entries `gone` names (one for each), and with `added`.
fn patched<T: PartialEq + Clone>(list: &mut Vec<T>, gone: &[T], added: &[T]) {
    for g in gone {
        if let Some(i) = list.iter().position(|e| e == g) {
            list.remove(i);
        }
    }
    list.extend(added.iter().cloned());
}

/// The lists of the content that people change: the items, the creatures, the quests, where the creatures stand and the chests.
#[derive(Clone, Debug, PartialEq)]
pub struct Raws {
    pub items: Vec<RawItem>,
    pub creatures: Vec<RawCreature>,
    pub quests: Vec<RawQuest>,
    pub spawns: Vec<Spawn>,
    pub chests: Vec<ChestDef>,
    pub pois: Vec<Poi>,
    pub props: Vec<Prop>,
    pub loot: BTreeMap<String, RawTable>,
}

impl Raws {
    /// What the content files hold.
    pub fn files() -> Result<Raws> {
        Ok(Raws {
            items: {
                let mut items: Vec<RawItem> = ron::from_str(&source("items.ron", include_str!("../../../content/items.ron"))).context("items.ron")?;
                items.extend(ron::from_str::<Vec<RawItem>>(&source("classic_items.ron", include_str!("../../../content/classic_items.ron"))).context("classic_items.ron")?);
                items.extend(ron::from_str::<Vec<RawItem>>(&source("classic_pieces.ron", include_str!("../../../content/classic_pieces.ron"))).context("classic_pieces.ron")?);
                items
            },
            creatures: {
                let mut creatures: Vec<RawCreature> = ron::from_str(&source("creatures.ron", include_str!("../../../content/creatures.ron"))).context("creatures.ron")?;
                let mut classic: Vec<RawCreature> = ron::from_str(&source("classic_creatures.ron", include_str!("../../../content/classic_creatures.ron"))).context("classic_creatures.ron")?;
                let mut brains: BTreeMap<String, Brain> = ron::from_str(&source("classic_creature_ai.ron", include_str!("../../../content/classic_creature_ai.ron"))).context("classic_creature_ai.ron")?;
                for c in &mut classic {
                    c.brain = brains.remove(&c.id).unwrap_or_default();
                }
                creatures.extend(classic);
                creatures
            },
            quests: ron::from_str(&source("quests.ron", include_str!("../../../content/quests.ron"))).context("quests.ron")?,
            spawns: ron::from_str(&source("spawns.ron", include_str!("../../../content/spawns.ron"))).context("spawns.ron")?,
            chests: ron::from_str(&source("chests.ron", include_str!("../../../content/chests.ron"))).context("chests.ron")?,
            pois: {
                let mut pois: Vec<Poi> = ron::from_str(&source("pois.ron", include_str!("../../../content/pois.ron"))).context("pois.ron")?;
                pois.extend(ron::from_str::<Vec<Poi>>(&source("classic_pois.ron", include_str!("../../../content/classic_pois.ron"))).context("classic_pois.ron")?);
                pois
            },
            props: {
                let mut props: Vec<Prop> = ron::from_str(&source("props.ron", include_str!("../../../content/props.ron"))).context("props.ron")?;
                props.extend(classic_props()?);
                props
            },
            loot: {
                let mut loot: BTreeMap<String, RawTable> = ron::from_str(&source("loot.ron", include_str!("../../../content/loot.ron"))).context("loot.ron")?;
                loot.extend(ron::from_str::<BTreeMap<String, RawTable>>(&source("classic_loot.ron", include_str!("../../../content/classic_loot.ron"))).context("classic_loot.ron")?);
                loot.extend(ron::from_str::<BTreeMap<String, RawTable>>(&source("classic_creature_loot.ron", include_str!("../../../content/classic_creature_loot.ron"))).context("classic_creature_loot.ron")?);
                loot
            },
        })
    }

    pub fn with(mut self, overlay: &Overlay) -> Raws {
        overlay.items.apply(&mut self.items);
        overlay.creatures.apply(&mut self.creatures);
        overlay.quests.apply(&mut self.quests);
        overlay.chests.apply(&mut self.chests);
        overlay.pois.apply(&mut self.pois);
        self.loot.retain(|k, _| !overlay.loot_removed.contains(k));
        self.loot.extend(overlay.loot.iter().map(|(k, v)| (k.clone(), v.clone())));
        patched(&mut self.spawns, &overlay.spawns_removed, &overlay.spawns_added);
        patched(&mut self.props, &overlay.props_removed, &overlay.props_added);
        self
    }
}

/// Parse and cross-check the content files and what the editor changed in them.
pub fn load(mount: Option<&Mount>) -> Result<Content> {
    let raws = Raws::files()?;
    // The tests are of the files themselves. An overlay that no longer fits the files (they changed under it) is left out, not fatal.
    if cfg!(test) {
        return build(raws, mount);
    }
    match Overlay::read().and_then(|overlay| build(raws.clone().with(&overlay), mount)) {
        Ok(content) => Ok(content),
        Err(e) => {
            log::warn!("content/editor.ron is left out: {e:#}");
            build(raws, mount)
        }
    }
}

/// Whether the content with these lists would be valid (the editor asks before it saves).
pub fn check(raws: &Raws) -> Result<()> {
    build(raws.clone(), None).map(|_| ())
}

/// Cross-check the lists and the rest of the content files and make the content of them.
pub fn build(raws: Raws, mount: Option<&Mount>) -> Result<Content> {
    let Raws { items: raw_items, creatures: raw_creatures, quests: raw_quests, spawns, chests, pois, props, loot: raw_loot } = raws;
    let display_dbc = mount.map(|m| -> Result<Dbc> { Ok(Dbc::parse(&m.read(r"DBFilesClient\ItemDisplayInfo.dbc")?)?) }).transpose().context("ItemDisplayInfo.dbc")?;
    let display = display_dbc.as_ref().zip(mount).map(|(dbc, mount)| Displays::new(dbc, mount));
    let spells: Vec<SpellDef> = ron::from_str(&source("spells.ron", include_str!("../../../content/spells.ron"))).context("spells.ron")?;
    for (i, s) in spells.iter().enumerate() {
        ensure!(spells[..i].iter().all(|o| o.id != s.id), "spell id {} is used twice", s.id);
        ensure!((1..=crate::skills::MAX).contains(&s.level) && s.class.is_class() && s.cooldown >= 0.0, "spell {}: level, class or cooldown", s.id);
    }
    let classic_spells: Vec<ClassicSpell> = ron::from_str(&source("classic_spells.ron", include_str!("../../../content/classic_spells.ron"))).context("classic_spells.ron")?;
    let mut seen = HashSet::new();
    ensure!(classic_spells.iter().all(|s| seen.insert(s.spell)), "classic_spells.ron: a spell is used twice");
    let classic_index: std::collections::HashMap<u32, usize> = classic_spells.iter().enumerate().map(|(i, s)| (s.spell, i)).collect();
    for c in &raw_creatures {
        for e in &c.brain.events {
            ensure!(e.acts.iter().all(|&(kind, spell, ..)| kind != crate::brain::act::CAST || classic_index.contains_key(&(spell as u32))), "creature {}: its script casts a spell that does not exist", c.id);
            ensure!(e.chance <= 100, "creature {}: the chance of an event is a percent", c.id);
        }
        ensure!(c.brain.mana >= 0.0 && c.brain.swing >= 0.0 && c.brain.shot >= 0.0 && c.brain.assist >= 0.0, "creature {}: mana, swing, shot or assist below zero", c.id);
    }
    let items = raw_items
        .into_iter()
        .map(|raw| {
            let id = raw.id.clone();
            build_item(raw, display.as_ref(), &spells).with_context(|| format!("item {id}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    if let Some(dup) = items.iter().find(|i| !seen.insert(i.id)) {
        bail!("item id {} is used twice", dup.id);
    }
    let item_numbers: std::collections::HashMap<&str, usize> = items.iter().enumerate().map(|(i, d)| (d.id, i)).collect();
    let item = |id: &str| item_numbers.get(id).copied().with_context(|| format!("unknown item {id}"));

    let loot = raw_loot
        .iter()
        .map(|(id, t)| {
            ensure!(t.copper.0 <= t.copper.1, "loot {id}: copper range");
            let mut copper = t.copper;
            let mut rows: Vec<&(String, (u32, u32), f32)> = t.items.iter().collect();
            for other in &t.also {
                let o = raw_loot.get(other).with_context(|| format!("loot {id}: unknown table {other}"))?;
                ensure!(o.also.is_empty(), "loot {id}: {other} rolls tables too; only one step is followed");
                copper = (copper.0 + o.copper.0, copper.1 + o.copper.1);
                rows.extend(o.items.iter());
            }
            let drops = rows
                .into_iter()
                .map(|(name, count, percent)| {
                    ensure!(count.0 >= 1 && count.0 <= count.1 && (0.0..=100.0).contains(percent), "loot {id}: {name} count or chance");
                    Ok((item(name).with_context(|| format!("loot {id}"))?, *count, *percent))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((id.clone(), DropTable { copper, items: drops }))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;

    let creatures = raw_creatures
        .into_iter()
        .map(|c| {
            let body = match c.body {
                RawBody::Beast { model, skins, scale } => Body::Beast { model, skins, scale },
                RawBody::Humanoid { outfit, look } => {
                    let outfit = outfit.iter().map(|id| item(id)).collect::<Result<Vec<_>>>().with_context(|| format!("creature {}", c.id))?;
                    ensure!(outfit.iter().all(|&i| items[i].wear != Wear::None), "creature {}: an item in its outfit cannot be worn", c.id);
                    ensure!(look.skin <= 9 && look.face <= 11 && look.hair_style <= 11 && look.hair_color <= 9 && look.beard <= 8, "creature {}: appearance", c.id);
                    Body::Humanoid { outfit, look }
                }
            };
            ensure!(c.health > 0.0 && c.level >= 1, "creature {}: health or level", c.id);
            let vendor = c.vendor.iter().map(|id| item(id)).collect::<Result<Vec<_>>>().with_context(|| format!("creature {}: wares", c.id))?;
            ensure!(vendor.iter().all(|&i| items[i].buy > 0), "creature {}: sells an item that has no price", c.id);
            let sells = c.gossip.as_ref().is_some_and(|g| g.options.iter().any(|o| matches!(o.action, Action::Vendor)));
            ensure!(sells == !vendor.is_empty(), "creature {}: a vendor line needs wares and wares need a vendor line", c.id);
            Ok(CreatureDef { id: c.id, name: c.name, level: c.level, title: c.title, body, hostile: c.hostile, neutral: c.neutral, health: c.health, bite_damage: c.bite_damage, respawn: c.respawn, loot: c.loot, gossip: c.gossip, vendor, brain: c.brain })
        })
        .collect::<Result<Vec<_>>>()?;
    for c in &creatures {
        if let Some(table) = &c.loot {
            ensure!(loot.contains_key(table), "creature {}: unknown loot table {table}", c.id);
        }
    }
    for s in &spawns {
        ensure!(creatures.iter().any(|c| c.id == s.creature), "spawn of unknown creature {}", s.creature);
    }

    let creature = |id: &str| creatures.iter().position(|c| c.id == id).with_context(|| format!("unknown creature {id}"));
    let mut quests: Vec<Quest> = Vec::new();
    for q in raw_quests {
        let id = q.id.clone();
        let build = || -> Result<Quest> {
            ensure!(!quests.iter().any(|o| o.id == q.id), "the id is used twice");
            let goals = q
                .goals
                .iter()
                .map(|g| match g {
                    RawGoal::Kill(name, count) => Ok(Goal::Kill { creature: creature(name)?, count: *count }),
                    RawGoal::Collect(name, count) => Ok(Goal::Collect { item: item(name)?, count: *count }),
                })
                .collect::<Result<Vec<_>>>()?;
            ensure!(goals.iter().all(|g| matches!(g, Goal::Kill { count, .. } | Goal::Collect { count, .. } if *count > 0)), "a goal of zero");
            let reward_items = q.reward.items.iter().map(|(name, count)| Ok(Stack { item: item(name)?, count: *count })).collect::<Result<Vec<_>>>()?;
            let choice = q.reward.choice.iter().map(|(name, count)| Ok(Stack { item: item(name)?, count: *count })).collect::<Result<Vec<_>>>()?;
            // The window has ten places for them.
            ensure!(reward_items.len() + choice.len() <= 10 && choice.len() != 1, "too many reward items, or a choice of one");
            let requires = q.requires.as_deref().map(|r| quests.iter().position(|o| o.id == r).with_context(|| format!("requires {r}, which must come earlier in the file"))).transpose()?;
            Ok(Quest { id: q.id.clone(), title: q.title.clone(), giver: creature(&q.giver)?, turn_in: creature(&q.turn_in)?, details: q.details.clone(), objective: q.objective.clone(), progress: q.progress.clone(), completion: q.completion.clone(), goals, copper: q.reward.copper, xp: q.reward.xp, items: reward_items, choice, requires })
        };
        let quest = build().with_context(|| format!("quest {id}"))?;
        quests.push(quest);
    }

    for c in &chests {
        ensure!(c.loot.is_empty() || loot.contains_key(&c.loot), "chest {}: unknown loot table {}", c.id, c.loot);
        ensure!(!c.loot.is_empty() || c.copper > 0 || !c.items.is_empty(), "chest {} holds nothing: give it a loot table, copper or items", c.id);
        for (id, count) in &c.items {
            ensure!(items.iter().any(|i| i.id == id.as_str()), "chest {}: unknown item {id}", c.id);
            ensure!(*count >= 1, "chest {}: {id} must be at least 1", c.id);
        }
    }

    for (i, p) in pois.iter().enumerate() {
        ensure!(p.radius > 0.0 && pois[..i].iter().all(|o| o.id != p.id), "poi {}: needs a radius and an id of its own", p.id);
    }

    let raw_start: RawStart = ron::from_str(&source("start.ron", include_str!("../../../content/start.ron"))).context("start.ron")?;
    let mut equipment = Vec::new();
    for (slot, id) in raw_start.equipment.iter() {
        let slot = *slot;
        let index = item(id)?;
        ensure!(slot < SLOTS && items[index].wear.fits(slot), "{id} cannot be worn in slot {slot}");
        equipment.push((slot, Stack { item: index, count: 1 }));
    }
    let bag = raw_start.bag.iter().map(|(id, count)| Ok(Stack { item: item(id)?, count: *count })).collect::<Result<Vec<_>>>()?;
    ensure!(bag.len() <= items::BACKPACK_SIZE, "the starting kit does not fit the backpack");
    let known = raw_start.spells.iter().map(|id| spells.iter().position(|s| s.id == *id).with_context(|| format!("start: unknown spell {id}"))).collect::<Result<Vec<_>>>()?;
    for p in &props {
        let model = p.model.to_ascii_lowercase();
        ensure!(model.ends_with(".m2") || model.ends_with(".wmo"), "prop {}: only .m2 and .wmo models can be placed", p.model);
        ensure!(p.scale > 0.0 && p.at.0.is_finite() && p.at.1.is_finite() && p.at.2.is_finite(), "prop {}: scale or place", p.model);
        if let Some(k) = &p.pickup {
            ensure!(items.iter().any(|i| i.id == k.item.as_str()) && k.respawn > 0.0 && model.ends_with(".m2"), "prop {}: its pickup needs an item of the game, a time and a model file", p.model);
        }
    }
    Ok(Content { spells, classic_spells, classic_index, items, pois, chests, quests, loot, creatures, spawns, props, start: Start { spells: known, money: raw_start.money, equipment, bag } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spells_of_the_original_are_all_there() {
        let fireball = get().classic_spells.iter().find(|s| s.spell == 133).expect("Fireball rank 1");
        assert_eq!((fireball.name.as_str(), fireball.cast, fireball.cost, fireball.effects[0].high), ("Fireball", 1500, 30, 22));
        assert!(get().classic_spells.len() > 20000);
    }

    #[test]
    fn the_content_files_are_valid_and_complete() {
        let c = get();
        assert!(c.items.len() >= 9 && !c.spawns.is_empty() && !c.start.bag.is_empty());
        // Every creature kind that drops something has its table, and every table has drops that make sense.
        assert!(c.creatures.iter().filter_map(|c| c.loot.as_ref()).all(|t| c.loot.contains_key(t)));
        assert!(c.loot.values().all(|t| t.items.iter().all(|&(i, (low, _), _)| low <= c.items[i].stack)));
    }

    #[test]
    fn every_quest_can_be_given_and_finished_by_creatures_that_exist_in_the_world() {
        let c = get();
        for q in &c.quests {
            // Both ends of the quest stand somewhere (a spawn of their kind), so the player can find them.
            for end in [q.giver, q.turn_in] {
                assert!(c.spawns.iter().any(|s| s.creature == c.creatures[end].id), "{} has nobody to talk to: {}", q.title, c.creatures[end].id);
                assert!(c.creatures[end].gossip.is_some() && !c.creatures[end].hostile, "{}: {} cannot be talked to", q.title, c.creatures[end].id);
            }
            // Kills are of hostile creatures and items to collect can actually be had: dropped by a creature or bought.
            for g in &q.goals {
                match *g {
                    Goal::Kill { creature, .. } => {
                        assert!(c.creatures[creature].attackable(), "{}: kills a creature that cannot be fought", q.title);
                        assert!(c.spawns.iter().any(|s| s.creature == c.creatures[creature].id), "{}: nobody of {} stands in the world", q.title, c.creatures[creature].id);
                    }
                    Goal::Collect { item, .. } => {
                        // Dropped by a creature that stands in the world (or a chest), or bought.
                        let dropped = c.creatures.iter().any(|cr| cr.loot.as_ref().is_some_and(|t| c.spawns.iter().any(|s| s.creature == cr.id) && c.loot[t].items.iter().any(|d| d.0 == item)));
                        let sold = c.creatures.iter().any(|cr| cr.vendor.contains(&item));
                        assert!(dropped || sold, "{}: {} cannot be had", q.title, c.items[item].name);
                    }
                }
            }
        }
    }

    #[test]
    fn the_chests_hold_something_and_the_player_can_buy_the_picks_to_open_them() {
        let c = get();
        assert!(c.chests.iter().any(|ch| ch.lock.is_some()), "a locked chest to pick");
        let pick = c.items.iter().position(|i| i.id == "lockpick").unwrap();
        // Picks can be had: bought, and found in the chests themselves.
        assert!(c.creatures.iter().any(|cr| cr.vendor.contains(&pick)));
        assert!(c.chests.iter().all(|ch| ch.items.iter().any(|(id, _)| id == "lockpick") || c.loot.get(&ch.loot).is_some_and(|t| t.items.iter().any(|d| d.0 == pick))), "every chest can give picks back");
    }

    #[test]
    fn the_items_that_quests_collect_are_the_quest_items_and_are_not_sold() {
        let c = get();
        let collected: Vec<usize> = c.quests.iter().flat_map(|q| &q.goals).filter_map(|g| if let Goal::Collect { item, .. } = g { Some(*item) } else { None }).collect();
        // What a quest asks to collect is a quest item (the original's quest items, of its own tables, are many more), and none is sold.
        assert!(collected.iter().all(|&i| c.items[i].quest), "a collected item is not flagged");
        assert!(c.items.iter().filter(|i| i.quest).all(|i| i.sell == 0));
    }

    #[test]
    fn a_bad_reference_is_an_error_not_a_crash() {
        let broken = ron::from_str::<RawStart>(r#"(money: 0, equipment: [(16, "nothing")], bag: [])"#).unwrap();
        assert_eq!(broken.equipment[0].1, "nothing");
        assert!(ron::from_str::<Vec<RawItem>>(r#"[(id: "x", name: "X", display: 1, flavour: 3)]"#).is_err(), "unknown fields are refused");
        let item = ron::from_str::<Vec<RawItem>>(r#"[(id: "x", name: "X", display: 1, stats: (1, 2, 3, 4, 5), weapon: Some((min: 1.0, max: 2.0, speed: 1.5)))]"#).unwrap();
        assert_eq!((item[0].quality, item[0].stack, item[0].stats.4), (1, 1, 5));
    }

    #[test]
    fn a_creatures_script_is_edited_in_the_overlay_and_comes_back() {
        let base = Raws::files().unwrap();
        let mut edited = base.clone();
        let mage = edited.creatures.iter_mut().find(|c| c.id == "c474").expect("Defias Rogue Wizard");
        assert!(!mage.brain.events.is_empty() && mage.brain.mana > 0.0, "the original's script is in the raw creature");
        mage.brain.mana = 999.0;
        mage.brain.events.pop();
        let overlay = Overlay::between(&base, &edited);
        assert_eq!(overlay.creatures.entries.len(), 1, "only what was edited");
        let back: Overlay = ron::from_str(&ron::to_string(&overlay).unwrap()).unwrap();
        assert_eq!(base.clone().with(&back), edited);
        check(&edited).unwrap();
        // A script that casts a spell that does not exist is refused.
        let mage = edited.creatures.iter_mut().find(|c| c.id == "c474").unwrap();
        mage.brain.events.push(BrainEvent { acts: vec![(crate::brain::act::CAST, 4_000_000, 1, 0)], ..Default::default() });
        assert!(check(&edited).is_err());
    }
}
