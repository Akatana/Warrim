//! The screens before the game, built from the original's own `GlueXML` frames: the list of characters (each one has a save of its own,
//! see `save.rs`) and the creation of a new one (race, sex, skin, face, hair, facial hair and a name), with the character standing
//! in the middle, dressed as its save says. The original draws a scene of the race's homeland behind it; here it is a plain backdrop.
//! This game has no classes (its people grow by skills), so the class buttons and the class text of the original are left out, and
//! every character starts in Northshire.

use crate::anim;
use crate::camera::Camera;
use crate::models::{ModelCache, ModelCpu, ModelVertex};
use crate::character::{self, Appearance, Race, Ranges, Sex};
use crate::creature::Person;
use crate::cursor::{Cursors, Kind};
use crate::renderer::{Gpu, Renderer};
use crate::save;
use crate::ui::{Quad, FULL_UV, WHITE};
use crate::uixml::{Point, Registry, Tree};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use wowfmt::m2::M2;

const SELECT: &str = "CharacterSelect";
const CREATE: &str = "CharacterCreate";
const DELETE: &str = "CharacterDeleteDialog";
/// How many characters the list holds (the original's buttons).
const MAX_CHARACTERS: usize = 10;
/// The races in the order of the buttons of the original: the Alliance's four, then the Horde's.
const RACES: [Race; 8] = [Race::Human, Race::Dwarf, Race::NightElf, Race::Gnome, Race::Orc, Race::Undead, Race::Tauren, Race::Troll];
/// Size the picture of the scene begins with (it takes the size of the window), and how fast the rotate buttons turn the character (radians a second).
const PICTURE: (u32, u32) = (1280, 720);
const TURN_SPEED: f32 = 2.1;
/// Two clicks on a character this close (seconds) are a double click.
const DOUBLE_CLICK: f32 = 0.4;
const GOLD: [f32; 4] = [1.0, 0.82, 0.0, 1.0];
const WHITE_TEXT: [f32; 4] = [1.0; 4];
const RED: [f32; 4] = [1.0, 0.1, 0.1, 1.0];

/// What the screens decided.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Quit,
    /// Play this character: one of the list, or the new one (whose save does not exist yet, and whose race has its introduction to see).
    Play { name: String, look: Appearance, new: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
    Backspace,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Select,
    Create,
}

struct Listed {
    name: String,
    look: Appearance,
    level: u32,
    outfit: Vec<usize>,
}

/// The scene behind the character: a model of the original (`Interface\Glues\Models\UI_*`), the camera it carries and the place (its first
/// attachment) where the character stands, facing along +x of the model.
struct Scene {
    entity: usize,
    m2: M2,
    template: Arc<ModelCpu>,
    camera: Camera,
    /// The model camera's field of view (see `camera::vertical_fov`).
    fov: f32,
    stand: Vec3,
    vertices: Vec<ModelVertex>,
}

/// The scene of a race (the gnomes have the dwarves' mountains, the trolls the orcs' valley).
fn scene_of(race: Race) -> &'static str {
    match race {
        Race::Human | Race::Goblin => r"Interface\Glues\Models\UI_Human\UI_Human.m2",
        Race::Dwarf | Race::Gnome => r"Interface\Glues\Models\UI_Dwarf\UI_Dwarf.m2",
        Race::NightElf => r"Interface\Glues\Models\UI_NightElf\UI_NightElf.m2",
        Race::Orc | Race::Troll => r"Interface\Glues\Models\UI_Orc\UI_Orc.m2",
        Race::Undead => r"Interface\Glues\Models\UI_Scourge\UI_Scourge.m2",
        Race::Tauren => r"Interface\Glues\Models\UI_Tauren\UI_Tauren.m2",
    }
}

/// The scene of the main menu of the original: no race's, with nobody standing in it (shown when there is no character to show).
const EMPTY_SCENE: &str = r"Interface\Glues\Models\UI_MainMenu\UI_MainMenu.m2";

/// Model space (x east, y north, z up) to render space.
fn render_point(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[2], -p[1])
}

impl Scene {
    fn load(mount: &Mount, renderer: &mut Renderer, gpu: &Gpu, file: &str) -> Result<Scene> {
        let m2 = M2::parse(&mount.read(file)?)?;
        let cam = m2.cameras.first().context("the scene has no camera")?.clone();
        let stand = m2.attachments.first().map_or([0.0; 3], |a| a.position);
        let template = ModelCache::default().creature(mount, file, &[])?;
        let entity = renderer.add_entity(gpu, &template);
        let (eye, target) = (render_point(cam.position), render_point(cam.target));
        let d = target - eye;
        let mut camera = Camera::new(eye, d.x.atan2(-d.z), d.y.atan2(d.x.hypot(d.z)));
        (camera.near, camera.far) = (cam.near, cam.far);
        Ok(Scene { entity, m2, template, camera, fov: cam.fov, stand: render_point(stand), vertices: Vec::new() })
    }
}

pub struct Glue {
    mount: Arc<Mount>,
    dir: PathBuf,
    tree: Tree,
    cursors: Option<Cursors>,
    screen: Screen,
    size: (u32, u32),
    mouse: (f32, f32),
    list: Vec<Listed>,
    selected: usize,
    last_click: Option<(usize, Instant)>,
    /// The look being made, the name being typed and what is wrong with it.
    look: Appearance,
    name: String,
    problem: Option<String>,
    /// The words typed to confirm a deletion, while the question is up.
    deleting: Option<String>,
    ranges: HashMap<(usize, usize, u32, u32), Ranges>,
    /// The character in the picture: what it is of, and how far it is turned (radians).
    shown: Option<(String, Person)>,
    target: (usize, usize),
    picture: (u32, u32),
    /// The scenes behind the character (by the file of the model), made when first wanted, and the one in the picture.
    scenes: HashMap<&'static str, Result<Scene, String>>,
    scene_now: Option<&'static str>,
    /// What the screens say with sounds, for the window to play, and the milliseconds the scene has been going.
    sounds: Vec<&'static str>,
    age: f32,
    turn: f32,
    /// Where the mouse went down on the background to turn the character with it.
    dragging: Option<f32>,
    seed: u64,
    start: Instant,
    outcome: Option<Outcome>,
}

/// A name as the original keeps it: a capital and small letters.
fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect())
}

/// What is wrong with a name (the key of the text that says it), if anything; `taken` are the names of the list.
fn name_problem(name: &str, taken: &[&str]) -> Option<&'static str> {
    let letters: Vec<char> = name.chars().collect();
    Some(match letters.len() {
        0 => "CHAR_NAME_NO_NAME",
        1 => "CHAR_NAME_TOO_SHORT",
        13.. => "CHAR_NAME_TOO_LONG",
        _ if !letters.iter().all(|c| c.is_alphabetic()) => "CHAR_NAME_INVALID_CHARACTER",
        _ if letters.windows(3).any(|w| w[0].to_lowercase().eq(w[1].to_lowercase()) && w[1].to_lowercase().eq(w[2].to_lowercase())) => "CHAR_NAME_THREE_CONSECUTIVE",
        // The names of the devices of Windows cannot be the names of files.
        _ if ["con", "prn", "aux", "nul"].contains(&name.to_lowercase().as_str()) => "CHAR_NAME_RESERVED",
        _ if taken.iter().any(|t| t.eq_ignore_ascii_case(name)) => "CHAR_CREATE_NAME_IN_USE",
        _ => return None,
    })
}

/// The part of the icon pictures a race (and sex) is on: columns and rows of four.
fn race_icon(race: Race, sex: Sex) -> [f32; 4] {
    let (col, row) = match race {
        Race::Human => (0, 0),
        Race::Dwarf => (1, 0),
        Race::Gnome => (2, 0),
        Race::NightElf => (3, 0),
        Race::Tauren => (0, 1),
        Race::Undead => (1, 1),
        Race::Troll => (2, 1),
        Race::Orc | Race::Goblin => (3, 1),
    };
    let row = row + if sex == Sex::Female { 2 } else { 0 };
    [col as f32 * 0.25, (col + 1) as f32 * 0.25, row as f32 * 0.25, (row + 1) as f32 * 0.25]
}

/// What facial hair is called for a race and sex (the key of the text).
fn facial_hair_key(look: &Appearance) -> &'static str {
    match (look.race, look.sex) {
        (Race::Human | Race::Dwarf | Race::Gnome, Sex::Male) | (Race::Orc, Sex::Male) => "FACIAL_HAIR_NORMAL",
        (Race::Human | Race::Dwarf | Race::Gnome, Sex::Female) => "FACIAL_HAIR_EARRINGS",
        (Race::NightElf, _) => "FACIAL_HAIR_MARKINGS",
        (Race::Orc, Sex::Female) => "FACIAL_HAIR_PIERCINGS",
        (Race::Troll, _) => "FACIAL_HAIR_TUSKS",
        _ => "FACIAL_HAIR_FEATURES",
    }
}

impl Glue {
    /// The screens, with the characters of `dir`; the first is shown, or the creation of one when there are none.
    pub fn new(mount: Arc<Mount>, renderer: &mut Renderer, gpu: &Gpu, dir: PathBuf, screen: Option<&str>) -> Result<Glue> {
        let mut tree = Tree::new(Rc::new(Registry::load_glue(&mount)?));
        for frame in [SELECT, CREATE, DELETE] {
            tree.instantiate(frame)?;
        }
        tree.show(DELETE, false);
        // No realms, no classes: those of the original's frames this game has nothing to put in.
        for part in ["CharSelectChangeRealmButton", "CharSelectRealmName", "CharacterCreateCharacterClass", "CharacterCreateRandomName"] {
            tree.show(part, false);
        }
        for i in 1..=8 {
            tree.show(&format!("CharacterCreateClassButton{i}"), false);
        }
        tree.set_text("CharacterSelectBackButtonText", &tree.string("QUIT"));
        // Where the class buttons were, the customizations begin.
        tree.set_point("CharacterCustomizationButtonFrame1", Point::Top, Some("CharacterCreateClassButton5"), Some(Point::Bottom), 0.0, 79.0);
        // The name box is tinted as the Alliance's (`CharacterCreate_OnLoad`).
        tree.set_backdrop_colors("CharacterCreateNameEdit", Some([0.09, 0.09, 0.19, 1.0]), Some([0.5, 0.5, 0.5, 1.0]));
        let list: Vec<Listed> = save::list(&dir).into_iter().take(MAX_CHARACTERS).map(|(name, s)| Listed { name, look: s.look, level: s.level(), outfit: s.outfit() }).collect();
        let cursors = Cursors::load(&mount, &mut renderer.ui, gpu).map_err(|e| log::warn!("no mouse pointer: {e:#}")).ok();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64) | 1;
        let target = renderer.add_offscreen(gpu, PICTURE);
        let mut glue = Glue {
            mount,
            dir,
            tree,
            cursors,
            screen: Screen::Select,
            size: (1280, 720),
            mouse: (-1.0, -1.0),
            list,
            selected: 0,
            last_click: None,
            look: Appearance::default(),
            name: String::new(),
            problem: None,
            deleting: None,
            ranges: HashMap::new(),
            shown: None,
            target,
            picture: PICTURE,
            scenes: HashMap::new(),
            scene_now: None,
            sounds: Vec::new(),
            age: 0.0,
            turn: 0.26,
            dragging: None,
            seed,
            start: Instant::now(),
            outcome: None,
        };
        match screen {
            Some("create") => glue.open_create(),
            Some(_) => glue.open_select(),
            None if glue.list.is_empty() => glue.open_create(),
            None => glue.open_select(),
        }
        Ok(glue)
    }

    pub fn outcome(&mut self) -> Option<Outcome> {
        self.outcome.take()
    }

    /// Whether the mouse pointer of the original is drawn (else the window's own is shown).
    pub fn has_cursor(&self) -> bool {
        self.cursors.is_some()
    }

    fn random(&mut self, below: u32) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        ((self.seed >> 33) % u64::from(below.max(1))) as u32
    }

    /// What the tables have to choose from for the look being made.
    fn ranges(&mut self) -> Ranges {
        let (look, mount) = (self.look, &self.mount);
        *self.ranges.entry((look.race as usize, look.sex as usize, look.skin, look.hair_style)).or_insert_with(|| {
            character::ranges(mount, look.race, look.sex, look.skin, look.hair_style).unwrap_or_else(|e| {
                log::warn!("no looks for {} {}: {e:#}", look.race.name(), look.sex.name());
                Ranges { skin: 0, face: 0, hair_style: 1, hair_color: 0, beard: 0 }
            })
        })
    }

    /// Keep every part of the look among those that exist (the faces depend on the skin, the hair colours on the style).
    fn fit(&mut self) {
        let r = self.ranges();
        self.look.skin = self.look.skin.min(r.skin);
        let r = self.ranges();
        self.look.face = self.look.face.min(r.face);
        self.look.hair_style = self.look.hair_style.clamp(1.min(r.hair_style), r.hair_style);
        let r = self.ranges();
        self.look.hair_color = self.look.hair_color.min(r.hair_color);
        self.look.beard = self.look.beard.min(r.beard);
    }

    /// A new person of this race and sex, plain.
    fn begin_look(&mut self, race: Race, sex: Sex) {
        self.look = Appearance { race, sex, skin: 0, face: 0, hair_style: 1, hair_color: 0, beard: 0, baked: 0 };
        self.fit();
    }

    fn randomize(&mut self) {
        let (race, sex) = (self.look.race, self.look.sex);
        self.begin_look(race, sex);
        let r = self.ranges();
        self.look.skin = self.random(r.skin + 1);
        let r = self.ranges();
        self.look.face = self.random(r.face + 1);
        self.look.hair_style = 1.min(r.hair_style) + self.random(r.hair_style + 1 - 1.min(r.hair_style));
        let r = self.ranges();
        self.look.hair_color = self.random(r.hair_color + 1);
        self.look.beard = self.random(r.beard + 1);
        self.fit();
    }

    /// Change one of the five customizations to the next or the previous that exists, round the ends.
    fn cycle(&mut self, which: usize, direction: i32) {
        let r = self.ranges();
        let (value, low, high) = match which {
            1 => (&mut self.look.skin, 0, r.skin),
            2 => (&mut self.look.face, 0, r.face),
            3 => (&mut self.look.hair_style, 1.min(r.hair_style), r.hair_style),
            4 => (&mut self.look.hair_color, 0, r.hair_color),
            _ => (&mut self.look.beard, 0, r.beard),
        };
        let count = (high + 1 - low) as i32;
        *value = low + (*value as i32 - low as i32 + direction).rem_euclid(count) as u32;
        self.fit();
    }

    fn open_select(&mut self) {
        self.screen = Screen::Select;
        self.deleting = None;
        self.selected = self.selected.min(self.list.len().saturating_sub(1));
        self.tree.show(SELECT, true);
        self.tree.show(CREATE, false);
        self.tree.show(DELETE, false);
        self.tree.show("CharSelectCreateCharacterButton", self.list.len() < MAX_CHARACTERS);
        self.tree.show("CharSelectEnterWorldButton", !self.list.is_empty());
        self.tree.show("CharacterSelectDeleteButton", !self.list.is_empty());
        for i in 1..=MAX_CHARACTERS {
            self.tree.show(&format!("CharSelectCharacterButton{i}"), i <= self.list.len());
        }
        for (i, c) in self.list.iter().enumerate() {
            let button = format!("CharSelectCharacterButton{}ButtonText", i + 1);
            self.tree.set_text(&format!("{button}Name"), &c.name);
            let info = self.tree.string("CHARACTER_SELECT_INFO").replacen("%d", &c.level.to_string(), 1).replacen("%s", c.look.race.name(), 1);
            self.tree.set_text(&format!("{button}Info"), &info);
            self.tree.set_text(&format!("{button}Location"), "");
        }
        self.mark_selected();
    }

    /// The selected character's name is white in the list and big above its picture.
    fn mark_selected(&mut self) {
        for i in 0..self.list.len() {
            self.tree.set_text_color(&format!("CharSelectCharacterButton{}ButtonTextName", i + 1), if i == self.selected { WHITE_TEXT } else { GOLD });
        }
        self.tree.set_text("CharSelectCharacterName", self.list.get(self.selected).map_or("", |c| &c.name));
    }

    fn open_create(&mut self) {
        self.screen = Screen::Create;
        self.tree.show(SELECT, false);
        self.tree.show(CREATE, true);
        self.tree.show(DELETE, false);
        self.name.clear();
        self.problem = None;
        for i in 1..=5 {
            let text = self.tree.string(&format!("CHAR_CUSTOMIZATION{i}_DESC"));
            self.tree.set_text(&format!("CharacterCustomizationButtonFrame{i}Text"), &text);
        }
        let (race, sex) = (self.look.race, self.look.sex);
        self.begin_look(race, sex);
        self.randomize();
        self.turn = 0.26;
        self.refresh();
    }

    /// Show the look being made in the texts, the icons and the buttons of the creation.
    fn refresh(&mut self) {
        let look = self.look;
        let tree = &mut self.tree;
        for (i, race) in RACES.iter().enumerate() {
            let button = format!("CharacterCreateRaceButton{}", i + 1);
            tree.set_region_coords(&format!("{button}NormalTexture"), race_icon(*race, look.sex));
            let on = *race == look.race;
            tree.set_text(&format!("{button}HighlightText"), if on { race.name() } else { "" });
            tree.set_checked(&button, on);
        }
        for (button, sex, coords) in [("Male", Sex::Male, [0.0, 0.5, 0.0, 1.0]), ("Female", Sex::Female, [0.5, 1.0, 0.0, 1.0])] {
            let name = format!("CharacterCreateGenderButton{button}");
            tree.set_region_coords(&format!("{name}NormalTexture"), coords);
            let on = sex == look.sex;
            tree.set_text(&format!("{name}HighlightText"), if on { if sex == Sex::Male { "Male" } else { "Female" } } else { "" });
            tree.set_checked(&name, on);
        }
        let horde = look.race.horde();
        // The boxes of the texts are tinted blue for the Alliance, red for the Horde (`SetCharacterRace`).
        let tint = if horde { [0.19, 0.05, 0.05, 1.0] } else { [0.09, 0.09, 0.19, 1.0] };
        for frame in ["CharacterCreateCharacterFaction", "CharacterCreateCharacterRace"] {
            tree.set_backdrop_colors(frame, Some(tint), None);
        }
        tree.set_region_coords("CharacterCreateFactionIcon", if horde { [0.5, 1.0, 0.0, 1.0] } else { [0.0, 0.5, 0.0, 1.0] });
        tree.set_text("CharacterCreateFactionLabel", if horde { "Horde" } else { "Alliance" });
        let faction = tree.string(if horde { "FACTION_INFO_HORDE" } else { "FACTION_INFO_ALLIANCE" });
        tree.set_text("CharacterCreateFactionText", &faction);
        tree.set_region_coords("CharacterCreateRaceIcon", race_icon(look.race, look.sex));
        tree.set_text("CharacterCreateRaceLabel", look.race.name());
        let key = look.race.key();
        let story = tree.string(&format!("RACE_INFO_{key}"));
        tree.set_text("CharacterCreateRaceText", &story);
        let mut abilities = String::new();
        for n in 1.. {
            let name = format!("ABILITY_INFO_{key}{n}");
            let text = tree.string(&name);
            if text == name {
                break;
            }
            abilities += &format!("{text}\n\n");
        }
        tree.set_text("CharacterCreateRaceAbilityText", &abilities);
        for scroll in ["CharacterCreateFactionScrollFrame", "CharacterCreateRaceScrollFrame"] {
            tree.reset_scroll(scroll);
        }
        let ranges = self.ranges();
        let tree = &mut self.tree;
        let hair = if look.race == Race::Tauren { "HORNS" } else { "NORMAL" };
        let (style, color) = (tree.string(&format!("HAIR_{hair}_STYLE")), tree.string(&format!("HAIR_{hair}_COLOR")));
        tree.set_text("CharacterCustomizationButtonFrame3Text", &style);
        tree.set_text("CharacterCustomizationButtonFrame4Text", &color);
        let facial = tree.string(facial_hair_key(&look));
        tree.set_text("CharacterCustomizationButtonFrame5Text", &facial);
        tree.show("CharacterCustomizationButtonFrame5", ranges.beard > 0);
    }

    // ---- the mouse and the keyboard

    pub fn mouse_moved(&mut self, pos: (f32, f32)) {
        self.mouse = pos;
        self.tree.hover = self.tree.hit_test(pos, self.size);
        if let Some(from) = self.dragging {
            self.turn += (pos.0 - from) * 0.012;
            self.dragging = Some(pos.0);
        }
    }

    pub fn mouse_down(&mut self, pos: (f32, f32)) {
        self.mouse = pos;
        let hit = self.tree.hit_test(pos, self.size);
        self.tree.pressed = hit;
        // The background turns the character.
        if hit.is_some_and(|h| matches!(self.tree.name(h), "CharacterCreateFrame" | "CharacterSelectUI")) {
            self.dragging = Some(pos.0);
        }
    }

    pub fn mouse_up(&mut self, pos: (f32, f32)) {
        self.dragging = None;
        let down = std::mem::take(&mut self.tree.pressed);
        let Some(hit) = self.tree.hit_test(pos, self.size).filter(|h| Some(*h) == down) else { return };
        let name = self.tree.name(hit).to_string();
        self.click(&name);
    }

    fn click(&mut self, name: &str) {
        if self.deleting.is_some() {
            match name {
                "CharacterDeleteButton1" => {
                    self.sounds.push("gsTitleOptionOK");
                    self.confirm_delete();
                }
                "CharacterDeleteButton2" => {
                    self.sounds.push("gsTitleOptionExit");
                    self.deleting = None;
                }
                _ => {}
            }
            self.tree.show(DELETE, self.deleting.is_some());
            return;
        }
        match (self.screen, name) {
            (Screen::Select, "CharacterSelectBackButton") => self.quit(),
            (Screen::Select, "CharSelectEnterWorldButton") => self.enter_world(),
            (Screen::Select, "CharSelectCreateCharacterButton") => {
                self.sounds.push("gsCharacterSelectionCreateNew");
                self.open_create();
            }
            (Screen::Select, "CharacterSelectDeleteButton") if !self.list.is_empty() => {
                self.sounds.push("gsCharacterSelectionDelCharacter");
                let c = &self.list[self.selected];
                let text = self.tree.string("CONFIRM_CHAR_DELETE").replacen("%s", &c.name, 1).replacen("%d", &c.level.to_string(), 1).replacen("%s", c.look.race.name(), 1);
                self.tree.set_text("CharacterDeleteText1", &text);
                self.tree.set_enabled("CharacterDeleteButton1", false);
                self.deleting = Some(String::new());
                self.tree.show(DELETE, true);
            }
            (Screen::Select, n) if n.starts_with("CharSelectCharacterButton") => {
                let Some(i) = n["CharSelectCharacterButton".len()..].parse::<usize>().ok().and_then(|i| i.checked_sub(1)).filter(|&i| i < self.list.len()) else { return };
                let double = self.last_click.is_some_and(|(was, at)| was == i && at.elapsed().as_secs_f32() < DOUBLE_CLICK);
                self.selected = i;
                self.sounds.push("gsCharacterSelection");
                self.last_click = Some((i, Instant::now()));
                self.mark_selected();
                if double {
                    self.enter_world();
                }
            }
            (Screen::Create, "CharCreateBackButton") => self.back_to_list(),
            (Screen::Create, "CharCreateOkayButton") => self.create(),
            (Screen::Create, "CharCreateRandomizeButton") => {
                self.sounds.push("gsCharacterCreationLook");
                self.randomize();
                self.refresh();
            }
            (Screen::Create, n) if n.starts_with("CharacterCreateRaceButton") => {
                if let Some(race) = n["CharacterCreateRaceButton".len()..].parse::<usize>().ok().and_then(|i| RACES.get(i.wrapping_sub(1))) {
                    self.sounds.push("gsCharacterCreationClass");
                    self.begin_look(*race, self.look.sex);
                    self.turn = 0.26;
                    self.refresh();
                }
            }
            (Screen::Create, "CharacterCreateGenderButtonMale" | "CharacterCreateGenderButtonFemale") => {
                self.sounds.push("gsCharacterCreationClass");
                self.begin_look(self.look.race, if name.ends_with("Female") { Sex::Female } else { Sex::Male });
                self.refresh();
            }
            (Screen::Create, n) if n.starts_with("CharacterCustomizationButtonFrame") => {
                let rest = &n["CharacterCustomizationButtonFrame".len()..];
                if let Some(which) = rest.get(..1).and_then(|d| d.parse::<usize>().ok()) {
                    match &rest[1..] {
                        "LeftButton" => self.cycle(which, -1),
                        "RightButton" => self.cycle(which, 1),
                        _ => return,
                    }
                    self.sounds.push("gsCharacterCreationLook");
                    self.refresh();
                }
            }
            _ => {}
        }
    }

    pub fn typed(&mut self, text: &str) {
        let letters = text.chars().filter(|c| !c.is_control());
        if let Some(word) = &mut self.deleting {
            word.extend(letters.take(32));
            word.truncate(32);
            let confirmed = word.eq_ignore_ascii_case("delete");
            self.tree.set_enabled("CharacterDeleteButton1", confirmed);
        } else if self.screen == Screen::Create {
            for c in letters.filter(|c| c.is_alphabetic()) {
                if self.name.chars().count() < 12 {
                    self.name.push(c);
                }
            }
            self.problem = None;
        }
    }

    pub fn key(&mut self, key: Key) {
        if let Some(word) = &mut self.deleting {
            match key {
                Key::Backspace => {
                    word.pop();
                    let confirmed = word.eq_ignore_ascii_case("delete");
                    self.tree.set_enabled("CharacterDeleteButton1", confirmed);
                }
                Key::Enter if word.eq_ignore_ascii_case("delete") => self.confirm_delete(),
                Key::Escape => self.deleting = None,
                Key::Enter => {}
            }
            self.tree.show(DELETE, self.deleting.is_some());
            return;
        }
        match (self.screen, key) {
            (Screen::Select, Key::Enter) => self.enter_world(),
            (Screen::Select, Key::Escape) => self.quit(),
            (Screen::Create, Key::Enter) => self.create(),
            (Screen::Create, Key::Escape) => self.back_to_list(),
            (Screen::Create, Key::Backspace) => {
                self.name.pop();
                self.problem = None;
            }
            (Screen::Select, Key::Backspace) => {}
        }
    }

    /// The sounds the screens made since the last call, by their names in `SoundEntries.dbc`.
    pub fn take_sounds(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.sounds)
    }

    fn quit(&mut self) {
        self.sounds.push("gsCharacterSelectionExit");
        self.outcome = Some(Outcome::Quit);
    }

    fn back_to_list(&mut self) {
        self.sounds.push("gsCharacterCreationCancel");
        self.open_select();
    }

    fn enter_world(&mut self) {
        if let Some(c) = self.list.get(self.selected) {
            self.sounds.push("gsCharacterSelectionEnterWorld");
            self.outcome = Some(Outcome::Play { name: c.name.clone(), look: c.look, new: false });
        }
    }

    fn create(&mut self) {
        let name = capitalised(&self.name);
        let taken: Vec<&str> = self.list.iter().map(|c| c.name.as_str()).collect();
        // A name of the folder that is not in the list (a save that cannot be read) is taken too.
        let on_disk = save::character_path(&name).exists() || self.dir.join(format!("{name}.ron")).exists();
        match name_problem(&name, &taken).or(on_disk.then_some("CHAR_CREATE_NAME_IN_USE")) {
            Some(key) => self.problem = Some(self.tree.string(key)),
            None => {
                self.sounds.push("gsCharacterCreationCreateChar");
                self.outcome = Some(Outcome::Play { name, look: self.look, new: true });
            }
        }
    }

    fn confirm_delete(&mut self) {
        self.deleting = None;
        let Some(c) = self.list.get(self.selected) else { return };
        if let Err(e) = save::delete(&self.dir.join(format!("{}.ron", c.name))) {
            log::warn!("{e:#}");
            return;
        }
        self.list.remove(self.selected);
        self.shown = None;
        self.open_select();
    }

    // ---- drawing

    /// Make the picture of the character (when what it shows changed) and queue everything for drawing.
    pub fn draw(&mut self, renderer: &mut Renderer, gpu: &Gpu, size: (u32, u32), dt: f32) {
        self.size = size;
        let held = self.tree.pressed.map(|p| self.tree.name(p).to_string());
        match held.as_deref() {
            Some("CharacterCreateRotateLeft" | "CharacterSelectRotateLeft") => self.turn -= TURN_SPEED * dt,
            Some("CharacterCreateRotateRight" | "CharacterSelectRotateRight") => self.turn += TURN_SPEED * dt,
            _ => {}
        }
        // The picture is as big as the window: the field of view of the scene's camera depends on its shape.
        if self.picture != size {
            renderer.resize_offscreen(gpu, self.target, size);
            self.picture = size;
        }
        self.render_character(renderer, gpu, dt);
        let (w, h) = (size.0 as f32, size.1 as f32);
        let ui = &mut renderer.ui;
        ui.quad(Quad { texture: WHITE, rect: [0.0, 0.0, w, h], uv: FULL_UV, color: [0.035, 0.04, 0.07, 1.0], additive: false });
        if self.scene_now.is_some() {
            ui.quad(Quad { texture: self.target.1, rect: [0.0, 0.0, w, h], uv: FULL_UV, color: [1.0; 4], additive: false });
        }
        self.tree.draw(ui, gpu, &self.mount, size);
        // What is typed is drawn by hand: into the name box, or the box of the question.
        let (box_name, text) = match &self.deleting {
            Some(word) => ("CharacterDeleteEditBox", Some(word.clone())),
            None if self.screen == Screen::Create => ("CharacterCreateNameEdit", Some(self.name.clone())),
            None => ("", None),
        };
        if let (Some(text), Some(r)) = (text, self.tree.rect_of(box_name, size)) {
            let blink = if (self.start.elapsed().as_millis() / 500) % 2 == 0 { "|" } else { "" };
            // Left aligned, 15 units in from the edge (the box's text insets), in the font of the original's edit boxes.
            let scale = h / crate::uixml::UI_HEIGHT;
            let at = ((r.x + 15.0) * scale, h - (r.y + r.h / 2.0) * scale - 13.0 * scale);
            self.tree.draw_text_aligned(ui, gpu, &self.mount, size, "GlueEditBoxFont", &format!("{text}{blink}"), at, 0.0, WHITE_TEXT);
        }
        if let (Some(problem), Screen::Create, Some(r)) = (&self.problem, self.screen, self.tree.rect_of("CharacterCreateNameEdit", size)) {
            let scale = h / crate::uixml::UI_HEIGHT;
            self.tree.draw_text_at_pixel(ui, gpu, &self.mount, size, "GlueFontRedLarge", problem, ((r.x + r.w / 2.0) * scale, h - (r.y + r.h) * scale - 90.0 * scale), RED);
        }
        if let Some(cursors) = self.cursors.as_ref().filter(|_| self.mouse.0 >= 0.0) {
            cursors.draw(ui, Kind::Point, self.mouse);
        }
    }

    /// The person to show: the character of the list that is selected, or the one being made.
    fn subject(&self) -> Option<(String, Appearance, Vec<usize>)> {
        let (look, outfit) = match self.screen {
            Screen::Create => (self.look, Vec::new()),
            Screen::Select => self.list.get(self.selected).map(|c| (c.look, c.outfit.clone()))?,
        };
        Some((format!("{look:?}|{outfit:?}"), look, outfit))
    }

    /// Draw the scene of the race with the character in it into the picture (the scene of the main menu when there is no character).
    fn render_character(&mut self, renderer: &mut Renderer, gpu: &Gpu, dt: f32) {
        self.age += dt * 1000.0;
        let subject = self.subject();
        let file = subject.as_ref().map_or(EMPTY_SCENE, |(_, look, _)| scene_of(look.race));
        match subject {
            Some((key, look, outfit)) if self.shown.as_ref().is_none_or(|(k, _)| *k != key) => {
                if let Some((_, old)) = self.shown.take() {
                    old.retire(renderer);
                }
                match Person::load(&outfit, &look, &self.mount, renderer, gpu, &format!("glue/{key}")) {
                    Ok(person) => self.shown = Some((key, person)),
                    Err(e) => log::warn!("cannot show the character: {e:#}"),
                }
            }
            Some(_) => {}
            None => {
                if let Some((_, old)) = self.shown.take() {
                    old.retire(renderer);
                }
            }
        }
        if !self.scenes.contains_key(file) {
            let made = Scene::load(&self.mount, renderer, gpu, file).map_err(|e| format!("{e:#}"));
            if let Err(e) = &made {
                log::warn!("no scene {file}: {e}");
            }
            self.scenes.insert(file, made);
        }
        let Some(Ok(scene)) = self.scenes.get_mut(file) else {
            self.scene_now = None;
            return;
        };
        self.scene_now = Some(file);
        // The parts of the scene take their places (the sky and the backdrops are turned by the bones), and move on.
        if let Some(seq) = scene.m2.sequences.iter().position(|q| q.id == 0) {
            let (start, length) = (scene.m2.sequences[seq].start as f32, (scene.m2.sequences[seq].end - scene.m2.sequences[seq].start).max(1) as f32);
            let bones = anim::bone_matrices(&scene.m2, seq, start + self.age % length, self.age);
            anim::skin(&scene.m2, &bones, &scene.template.vertices, &mut scene.vertices);
            renderer.update_entity_vertices(gpu, scene.entity, &scene.vertices);
        }
        // The clouds and the water of the scene move.
        if let Some(tracks) = scene.template.tracks.as_ref().filter(|t| t.animated) {
            let looks: Vec<_> = scene.template.batches.iter().map(|b| tracks.look(b.refs, self.age)).collect();
            renderer.set_entity_looks(gpu, scene.entity, &looks);
        }
        renderer.set_entity(scene.entity, Vec3::ZERO, Mat4::IDENTITY, true);
        let mut ids = vec![scene.entity];
        if let Some((_, person)) = &mut self.shown {
            // The character faces the camera (along +x of the model), turned as far as the player turned it.
            ids.extend(person.stand_facing(dt, scene.stand, std::f32::consts::FRAC_PI_2 + self.turn, renderer, gpu));
        }
        scene.camera.aspect = self.picture.0 as f32 / self.picture.1 as f32;
        scene.camera.fov_y = crate::camera::vertical_fov(scene.fov, scene.camera.aspect);
        renderer.render_offscreen(gpu, self.target.0, &scene.camera, &ids, std::iter::empty());
        // Only the picture shows them.
        for id in ids {
            renderer.set_entity(id, Vec3::ZERO, Mat4::IDENTITY, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_letters_and_not_taken() {
        let taken = ["Thrall"];
        assert_eq!(name_problem("", &taken), Some("CHAR_NAME_NO_NAME"));
        assert_eq!(name_problem("A", &taken), Some("CHAR_NAME_TOO_SHORT"));
        assert_eq!(name_problem("Abcdefghijklm", &taken), Some("CHAR_NAME_TOO_LONG"));
        assert_eq!(name_problem("Gr0m", &taken), Some("CHAR_NAME_INVALID_CHARACTER"));
        assert_eq!(name_problem("Baaad", &taken), Some("CHAR_NAME_THREE_CONSECUTIVE"));
        assert_eq!(name_problem("Con", &taken), Some("CHAR_NAME_RESERVED"));
        assert_eq!(name_problem("tHRALL", &taken), Some("CHAR_CREATE_NAME_IN_USE"));
        assert_eq!(name_problem("Garrosh", &taken), None);
        assert_eq!(capitalised("gARROSH"), "Garrosh");
    }

    #[test]
    fn every_race_has_its_own_icon_and_the_women_are_below_the_men() {
        let mut seen = Vec::new();
        for race in Race::ALL {
            let (m, f) = (race_icon(race, Sex::Male), race_icon(race, Sex::Female));
            assert_eq!((m[0], m[1]), (f[0], f[1]));
            assert_eq!(f[2], m[2] + 0.5);
            assert!(!seen.contains(&m), "{race:?}");
            seen.push(m);
        }
        assert_eq!(RACES.len(), Race::ALL.len());
        assert!(Race::ALL.iter().all(|r| RACES.contains(r)));
    }
}
