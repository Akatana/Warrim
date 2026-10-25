//! The saved game: where the character stands, what it carries and wears, its level, its quests, and the world as it
//! is: the time of day and every creature and chest (where, how hurt, what it is doing, what it holds). It is the
//! game's state, not a copy of the process memory (that holds GPU handles, audio streams and pointers into the
//! content, which mean nothing in the next run). Items, quests and kinds are saved by their ids (not their numbers in
//! the content files), so a save survives changes to the content: what no longer exists is dropped when the game is
//! loaded, and what no longer fits where it was goes in the backpack. Creatures and chests are matched by their place
//! in the spawn list, kind and home; those that do not match start afresh.

use crate::character::Appearance;
use crate::chest::ChestSave;
use crate::content;
use crate::creature::MobSave;
use crate::items::{self, Inventory, Stack, BACKPACK_SIZE, SLOTS};
use crate::levels::{Progress, MAX_LEVEL};
use crate::loot::Loot;
use crate::quests::{Active, QuestLog, MAX_ACTIVE};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;

/// The furthest backpack slot a save may use.
const MAX_BAG_SLOT: usize = 1 << 16;

/// An item on a stack as saved: where it is (a slot), its id and how many.
type Entry = (usize, String, u32);

/// What a corpse or a chest holds as saved: the copper and the items by id.
pub type LootSave = (u32, Vec<(String, u32)>);

pub fn loot_to_save(loot: &Loot) -> LootSave {
    (loot.copper, loot.items.iter().map(|s| (items::get(s.item).id.to_string(), s.count)).collect())
}

/// Back from a save; items the content no longer has are dropped.
pub fn loot_from_save((copper, stacks): &LootSave) -> Loot {
    let items = stacks
        .iter()
        .filter_map(|(id, n)| content::get().items.iter().position(|i| i.id == *id).map(|item| Stack { item, count: (*n).clamp(1, items::get(item).stack) }))
        .collect();
    Loot { copper: *copper, items }
}

fn full() -> f32 {
    1.0
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Save {
    version: u32,
    /// Where the character stands: WoW coordinates (north, west), the heading in degrees (0 = north) and the pitch.
    pub position: (f32, f32),
    pub heading: f32,
    pub pitch: f32,
    /// The share of its health the character has left.
    pub health: f32,
    level: u32,
    xp: u32,
    money: u32,
    equipment: Vec<Entry>,
    bag: Vec<Entry>,
    /// The quests being worked on with the kills for each goal, and the finished ones.
    quests: Vec<(String, Vec<u32>)>,
    done: Vec<String>,
    /// The quest the compass points to (id); older saves have none.
    #[serde(default)]
    tracked: Option<String>,
    /// Ids of the places discovered (`pois.ron`); older saves have none.
    #[serde(default)]
    pub discovered: Vec<String>,
    /// `AreaTable` ids the character has been in (the overlays of the world map).
    #[serde(default)]
    pub explored: Vec<u32>,
    /// The rank of each skill and the uses towards the next one; the spells known, in the order they were learned.
    #[serde(default)]
    pub skills: Vec<(String, u32, f32)>,
    #[serde(default)]
    pub spells: Vec<String>,
    /// What is on each action button ("" for none); older saves have none and get the basic actions and the spells in order.
    #[serde(default)]
    pub bar: Vec<String>,
    /// The world: the hour of the day (none when the game ran at a fixed time), the share left of mana and stamina, the spells
    /// still cooling down (id, seconds), and every creature and chest in spawn order.
    #[serde(default)]
    pub hour: Option<f32>,
    #[serde(default = "full")]
    pub mana: f32,
    #[serde(default = "full")]
    pub stamina: f32,
    #[serde(default)]
    pub cooldowns: Vec<(String, f32)>,
    #[serde(default)]
    pub mobs: Vec<MobSave>,
    #[serde(default)]
    pub chests: Vec<ChestSave>,
    /// The steed being ridden (its display row and speed bonus in percent); none on foot.
    #[serde(default)]
    pub steed: Option<(u32, i32)>,
    /// The weapons are put away (R).
    #[serde(default)]
    pub sheathed: bool,
    /// Height of the feet, so a character saved on a bridge or a building floor comes back there, not on the terrain below.
    #[serde(default)]
    pub height: Option<f32>,
    /// What works on the character: buffs, item cooldowns (id, seconds), the seconds left of the way home; and the sneak.
    #[serde(default)]
    pub buffs: Vec<crate::consume::BuffSave>,
    #[serde(default)]
    pub item_cooldowns: Vec<(String, f32)>,
    #[serde(default)]
    pub hearth: Option<f32>,
    #[serde(default)]
    pub sneaking: bool,
    /// Who the character is: its race, sex and looks (a save from before the character list is a human man).
    #[serde(default)]
    pub look: Appearance,
    /// The map the character is on (a dungeon, say); none in older saves: the map the game starts on.
    #[serde(default)]
    pub map: Option<String>,
    /// The level-up points spent on health, mana and stamina (what is unspent follows from the level).
    #[serde(default)]
    pub spent: [u32; 3],
    /// The talents learned: the id and the rank of each.
    #[serde(default)]
    pub talents: Vec<(String, u8)>,
    /// The rest of the world: the hour is held (the console's `time`); the weather of the zones (zone, kind, grade, seconds to the next roll); the
    /// props to pick up that are gone (place, seconds until they are back); the physics props that were moved; the seconds on the lifts' and ships' clock.
    #[serde(default)]
    pub time_fixed: bool,
    #[serde(default)]
    pub weather: Vec<(u32, u8, f32, f32)>,
    #[serde(default)]
    pub gone: Vec<(usize, f32)>,
    /// The nodes (veins and herbs) that are gone: their place in the map's list, seconds until they are back.
    #[serde(default)]
    pub nodes_gone: Vec<(usize, f32)>,
    #[serde(default)]
    pub moved: Vec<crate::physics::BodySave>,
    #[serde(default)]
    pub lifts_clock: f32,
}

fn entries(slots: &[Option<Stack>]) -> Vec<Entry> {
    slots.iter().enumerate().filter_map(|(slot, s)| s.map(|s| (slot, items::get(s.item).id.to_string(), s.count))).collect()
}

impl Save {
    pub fn level(&self) -> u32 {
        self.level
    }

    /// What the character wears, as items of the content (for its picture in the list).
    pub fn outfit(&self) -> Vec<usize> {
        self.equipment.iter().filter_map(|(_, id, _)| content::get().items.iter().position(|i| i.id == *id)).collect()
    }

    /// Everything that is saved, from the game's parts. What the mouse carries is put in the backpack's first free slot.
    pub fn capture(inv: &Inventory, progress: Progress, quests: &QuestLog, position: (f32, f32), heading: f32, pitch: f32, health: f32) -> Save {
        let mut bag = inv.bag.clone();
        if let Some(carried) = inv.cursor {
            match bag.iter().position(Option::is_none) {
                Some(free) => bag[free] = Some(carried),
                None => bag.push(Some(carried)),
            }
        }
        Save {
            version: VERSION,
            position,
            heading,
            pitch,
            health,
            level: progress.level,
            xp: progress.xp,
            money: inv.money,
            equipment: entries(&inv.equipment),
            bag: entries(&bag),
            quests: quests.active.iter().map(|a| (content::quest(a.quest).id.clone(), a.kills.clone())).collect(),
            done: quests.done.iter().map(|&q| content::quest(q).id.clone()).collect(),
            tracked: quests.tracked.map(|q| content::quest(q).id.clone()),
            discovered: Vec::new(),
            explored: Vec::new(),
            skills: Vec::new(),
            spells: Vec::new(),
            bar: Vec::new(),
            hour: None,
            mana: 1.0,
            stamina: 1.0,
            cooldowns: Vec::new(),
            mobs: Vec::new(),
            chests: Vec::new(),
            steed: None,
            sheathed: false,
            height: None,
            buffs: Vec::new(),
            item_cooldowns: Vec::new(),
            hearth: None,
            sneaking: false,
            look: Appearance::default(),
            map: None,
            spent: [0; 3],
            talents: Vec::new(),
            time_fixed: false,
            weather: Vec::new(),
            gone: Vec::new(),
            nodes_gone: Vec::new(),
            moved: Vec::new(),
            lifts_clock: 0.0,
        }
    }

    /// Put the saved character into the game's parts, forgiving what the content no longer has. Returns what had to be
    /// dropped or moved, for the log.
    pub fn apply(&self, inv: &mut Inventory, progress: &mut Progress, quests: &mut QuestLog) -> Vec<String> {
        let mut notes = Vec::new();
        let find = |id: &str| content::get().items.iter().position(|i| i.id == id);
        *inv = Inventory { equipment: [None; SLOTS], bag: vec![None; BACKPACK_SIZE], cursor: None, money: self.money };
        // A stack the way it can exist: a known item, at least one and at most a full stack.
        let stack = |id: &str, count: u32| find(id).map(|item| Stack { item, count: count.clamp(1, items::get(item).stack) });
        let mut homeless = Vec::new();
        for (slot, id, count) in &self.equipment {
            match stack(id, *count) {
                Some(s) if *slot < SLOTS && items::get(s.item).wear.fits(*slot) && inv.equipment[*slot].is_none() => inv.equipment[*slot] = Some(s),
                Some(s) => homeless.push(s),
                None => notes.push(format!("item {id} no longer exists")),
            }
        }
        for (slot, id, count) in &self.bag {
            match stack(id, *count) {
                // shortcut: a slot beyond MAX_BAG_SLOT (a damaged save) is not trusted to size the pack; the stack is put in the first free slot.
                Some(s) if *slot < MAX_BAG_SLOT && inv.bag.get(*slot).is_none_or(Option::is_none) => {
                    if inv.bag.len() <= *slot {
                        inv.bag.resize(*slot + 1, None);
                    }
                    inv.bag[*slot] = Some(s);
                }
                Some(s) => homeless.push(s),
                None => notes.push(format!("item {id} no longer exists")),
            }
        }
        for s in homeless {
            if !inv.add(s) {
                notes.push(format!("no room for {}", items::get(s.item).name));
            }
        }
        inv.tidy();

        *progress = Progress { level: self.level.clamp(1, MAX_LEVEL), xp: 0 };
        progress.xp = if progress.at_max() { 0 } else { self.xp.min(progress.to_next().saturating_sub(1)) };

        let quest = |id: &str| content::get().quests.iter().position(|q| q.id == id);
        *quests = QuestLog::default();
        for (id, kills) in &self.quests {
            let Some(q) = quest(id) else {
                notes.push(format!("quest {id} no longer exists"));
                continue;
            };
            if quests.active.len() >= MAX_ACTIVE || quests.is_active(q) {
                continue;
            }
            let goals = &content::quest(q).goals;
            let kills = (0..goals.len()).map(|g| kills.get(g).copied().unwrap_or(0)).collect();
            quests.active.push(Active { quest: q, kills });
        }
        for id in &self.done {
            match quest(id) {
                Some(q) if !quests.done.contains(&q) && !quests.is_active(q) => quests.done.push(q),
                Some(_) => {}
                None => notes.push(format!("quest {id} no longer exists")),
            }
        }
        quests.tracked = self.tracked.as_deref().and_then(quest).filter(|&q| quests.is_active(q));
        notes
    }
}

/// Where the saves are kept: the user's application data folder (`rsworld`), or the current folder without one.
fn base_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    base.map_or_else(|| PathBuf::from("."), |b| b.join("rsworld"))
}

/// The options kept in a file of the application folder (not of a character); the defaults when there is none or it cannot be read.
pub fn load_settings<T: serde::de::DeserializeOwned + Default>(file: &str) -> T {
    std::fs::read_to_string(base_dir().join(file)).ok().and_then(|t| ron::from_str(&t).ok()).unwrap_or_default()
}

pub fn write_settings<T: Serialize>(file: &str, options: &T) {
    let path = base_dir().join(file);
    let written = ron::ser::to_string_pretty(options, ron::ser::PrettyConfig::default())
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(std::fs::create_dir_all(base_dir())?).and_then(|_| Ok(std::fs::write(&path, text)?)));
    if let Err(e) = written {
        log::warn!("cannot save {file}: {e:#}");
    }
}

/// The folder of the characters: each one is a file named after it (`Thrall.ron`).
pub fn characters_dir() -> PathBuf {
    base_dir().join("characters")
}

pub fn character_path(name: &str) -> PathBuf {
    characters_dir().join(format!("{name}.ron"))
}

/// The characters of a folder, the one played last first: (name, save). A save that cannot be read is left alone and not listed.
/// The single save of the days before the list (`save.ron` beside the folder) becomes a character named Adventurer.
pub fn list(dir: &Path) -> Vec<(String, Save)> {
    let old = dir.with_file_name("save.ron");
    if old.is_file() && !dir.join("Adventurer.ron").exists() && std::fs::create_dir_all(dir).is_ok() {
        if let Err(e) = std::fs::rename(&old, dir.join("Adventurer.ron")) {
            log::warn!("cannot move {} into the character list: {e}", old.display());
        }
    }
    let Ok(files) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<_> = files
        .filter_map(Result::ok)
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .filter_map(|p| {
            let name = p.file_stem()?.to_str()?.to_string();
            let saved = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            match read(&p) {
                Ok(Some(save)) => Some((saved, name, save)),
                Ok(None) => None,
                Err(e) => {
                    log::warn!("{e:#}");
                    None
                }
            }
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, name, save)| (name, save)).collect()
}

/// Delete a character: its file is renamed (`Thrall.ron.deleted`), so that a mistake can still be undone by hand.
pub fn delete(path: &Path) -> Result<()> {
    std::fs::rename(path, path.with_extension("ron.deleted")).with_context(|| format!("deleting {}", path.display()))
}

/// Read a save. `Ok(None)` when there is none yet; an error when it cannot be read (the file is left alone).
pub fn read(path: &Path) -> Result<Option<Save>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let save: Save = ron::from_str(&text).with_context(|| format!("{} is not a save of this game", path.display()))?;
    anyhow::ensure!(save.version == VERSION, "{} is a save of another version ({})", path.display(), save.version);
    Ok(Some(save))
}

/// Write a save: to a file beside it first and then over the old one, so that a crash never leaves half a save.
pub fn write(path: &Path, save: &Save) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = ron::ser::to_string_pretty(save, ron::ser::PrettyConfig::default())?;
    let temp = path.with_extension("ron.tmp");
    std::fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quest(id: &str) -> usize {
        content::quest_index(id)
    }

    /// A character with a story: worn and carried items, a level, a quest half done and one finished.
    fn story() -> (Inventory, Progress, QuestLog) {
        let mut inv = Inventory::new();
        inv.money = 4321;
        assert!(inv.equip_from_bag(1, 60).is_ok());
        inv.cursor = Some(Stack { item: items::find("tough_wolf_meat"), count: 7 });
        let mut log = QuestLog::default();
        log.accept(quest("q7"));
        log.killed(content::creature_index("c6"));
        log.done.push(quest("q783"));
        (inv, Progress { level: 5, xp: 1234 }, log)
    }

    #[test]
    fn a_saved_game_comes_back_as_it_was() {
        let (inv, progress, log) = story();
        let save = Save::capture(&inv, progress, &log, (-8950.0, -132.5), 200.0, -5.0, 0.75);
        let text = ron::ser::to_string_pretty(&save, ron::ser::PrettyConfig::default()).unwrap();
        let back: Save = ron::from_str(&text).unwrap();
        assert_eq!(back, save);
        let (mut inv2, mut progress2, mut log2) = (Inventory::new(), Progress::new(), QuestLog::default());
        assert!(back.apply(&mut inv2, &mut progress2, &mut log2).is_empty());
        assert_eq!((inv2.equipment, inv2.money, progress2), (inv.equipment, 4321, progress));
        // What the mouse carried is in the pack now; everything else is where it was.
        let mut expected = Inventory { equipment: inv.equipment, bag: inv.bag.clone(), cursor: None, money: 0 };
        let free = expected.bag.iter().position(Option::is_none).unwrap();
        expected.bag[free] = inv.cursor;
        expected.tidy();
        assert_eq!((inv2.bag, inv2.cursor), (expected.bag, None));
        assert_eq!((log2.active, log2.done, log2.tracked), (log.active, log.done, log.tracked));
        assert_eq!((back.position, back.heading, back.pitch, back.health), ((-8950.0, -132.5), 200.0, -5.0, 0.75));
    }

    #[test]
    fn what_the_content_no_longer_has_is_dropped_and_what_does_not_fit_moves_to_the_pack() {
        let text = r#"(version: 1, position: (0.0, 0.0), heading: 0.0, pitch: 0.0, health: 1.0, level: 999, xp: 999999, money: 5,
            equipment: [(16, "longsword", 1), (5, "round_shield", 1), (3, "does_not_exist", 1), (99, "hatchet", 1)],
            bag: [(0, "linen_cloth", 500), (0, "scuffed_boots", 1), (77, "hearthstone", 1)],
            quests: [("q7", [9, 4]), ("q7", [1]), ("no_such_quest", [])],
            done: ["q7", "also_missing"])"#;
        let save: Save = ron::from_str(text).unwrap();
        let (mut inv, mut progress, mut log) = (Inventory::new(), Progress::new(), QuestLog::default());
        let notes = save.apply(&mut inv, &mut progress, &mut log);
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert_eq!(inv.equipment[items::MAIN_HAND].map(|s| items::get(s.item).id), Some("longsword"));
        // A shield in the chest slot and a hatchet in a slot that does not exist go to the backpack instead.
        assert_eq!(inv.equipment[items::CHEST], None);
        let carried: Vec<&str> = inv.bag.iter().flatten().map(|s| items::get(s.item).id).collect();
        for id in ["round_shield", "hatchet", "scuffed_boots", "hearthstone"] {
            assert!(carried.contains(&id), "{id} in {carried:?}");
        }
        // A stack of 500 is a full stack of 20; the level and experience are held to what exists.
        assert_eq!(inv.bag[0], Some(Stack { item: items::find("linen_cloth"), count: 20 }));
        assert_eq!(progress, Progress { level: MAX_LEVEL, xp: 0 });
        // The quest is taken once, with its kills made to fit; being both active and done counts as active.
        assert_eq!(log.active, vec![Active { quest: quest("q7"), kills: vec![9] }]);
        assert!(log.done.is_empty());
    }

    #[test]
    fn loot_survives_a_save_and_loses_only_what_the_content_no_longer_has() {
        let meat = items::find("tough_wolf_meat");
        let loot = Loot { copper: 9, items: vec![Stack { item: meat, count: 2 }] };
        assert_eq!(loot_from_save(&loot_to_save(&loot)), loot);
        let old = (5, vec![("does_not_exist".to_string(), 1), ("tough_wolf_meat".to_string(), 3)]);
        assert_eq!(loot_from_save(&old), Loot { copper: 5, items: vec![Stack { item: meat, count: 3 }] });
        // A save from before the world was saved has none of it, and then everything is full and fresh.
        let save: Save = ron::from_str(r#"(version: 1, position: (0.0, 0.0), heading: 0.0, pitch: 0.0, health: 1.0, level: 1, xp: 0, money: 0, equipment: [], bag: [], quests: [], done: [])"#).unwrap();
        assert_eq!((save.hour, save.mana, save.stamina, save.mobs.len(), save.chests.len(), save.steed), (None, 1.0, 1.0, 0, 0, None));
        // A rider is saved riding.
        let ridden: Save = ron::from_str(&ron::to_string(&Save { steed: Some((2404, 60)), ..save }).unwrap()).unwrap();
        assert_eq!(ridden.steed, Some((2404, 60)));
        assert!(!ridden.sheathed, "weapons are drawn unless saved away");
        let away: Save = ron::from_str(&ron::to_string(&Save { sheathed: true, ..ridden }).unwrap()).unwrap();
        assert!(away.sheathed);
    }

    #[test]
    fn a_missing_file_is_a_new_game_and_a_broken_one_an_error_that_leaves_it_alone() {
        let dir = std::env::temp_dir().join(format!("rsworld-save-test-{}", std::process::id()));
        let path = dir.join("save.ron");
        assert!(read(&path).unwrap().is_none());
        let (inv, progress, log) = story();
        let save = Save::capture(&inv, progress, &log, (1.0, 2.0), 3.0, 4.0, 0.5);
        write(&path, &save).unwrap();
        assert_eq!(read(&path).unwrap().as_ref(), Some(&save));
        assert!(!path.with_extension("ron.tmp").exists(), "no half-written file is left beside it");
        std::fs::write(&path, "this is not a save").unwrap();
        assert!(read(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is not a save");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_character_has_its_file_and_the_old_single_save_becomes_one() {
        let base = std::env::temp_dir().join(format!("rsworld-list-test-{}", std::process::id()));
        let dir = base.join("characters");
        let (inv, progress, log) = story();
        let mut save = Save::capture(&inv, progress, &log, (1.0, 2.0), 3.0, 4.0, 0.5);
        save.look = Appearance { race: crate::character::Race::Orc, ..Appearance::default() };
        write(&base.join("save.ron"), &save).unwrap();
        write(&dir.join("Thrall.ron"), &save).unwrap();
        let names: Vec<String> = list(&dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Adventurer".to_string()) && names.contains(&"Thrall".to_string()));
        assert!(!base.join("save.ron").exists());
        assert_eq!(list(&dir)[0].1.look.race, crate::character::Race::Orc);
        delete(&dir.join("Thrall.ron")).unwrap();
        assert_eq!(list(&dir).len(), 1);
        assert!(dir.join("Thrall.ron.deleted").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
