//! The game menu (Esc, `GameMenuFrame`) and the options it opens: the video options (`OptionsFrame`), the sound options (`SoundOptionsFrame`)
//! and the interface options (`UIOptionsFrame`), all the original's frames with only what this game has left in them. The key bindings window is
//! `keybinds.rs`; the macros are in the menu but greyed out: there is nothing behind them.

use super::*;
use crate::audio::SoundSettings;
use crate::interface::InterfaceSettings;
use crate::video::{DisplayMode, VideoSettings};

const GAME_MENU: &str = "GameMenuFrame";
const SOUND: &str = "SoundOptionsFrame";
const VIDEO: &str = "OptionsFrame";
const INTERFACE: &str = "UIOptionsFrame";
const SOUND_CHECK: &str = "SoundOptionsFrameCheckButton";
/// The sliders of the windows, and how long they are (`OptionsSliderTemplate`).
const SLIDERS: [&str; 13] = [
    "SoundOptionsFrameSlider1",
    "SoundOptionsFrameSlider2",
    "SoundOptionsFrameSlider3",
    "SoundOptionsFrameSlider4",
    "OptionsFrameSlider1",
    "OptionsFrameSlider2",
    "OptionsFrameSlider3",
    "OptionsFrameSlider6",
    "OptionsFrameSlider7",
    "OptionsFrameSlider8",
    "OptionsFrameSlider9",
    "UIOptionsFrameSlider3",
    "UIOptionsFrameSlider4",
];
pub(super) const SLIDER_WIDTH: f32 = 128.0;
/// The slider of the interface scale, which the interface follows once it is let go (it would run away from the pointer).
const SCALE_SLIDER: &str = "OptionsFrameSlider1";
/// The list that the dropdowns open (`DropDownList1`).
const LIST: &str = "DropDownList1";
const LIST_ROW: f32 = 16.0;
const LIST_MAX: usize = 32;

/// The dropdowns of the video options.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dropdown {
    Mode,
    Resolution,
    Refresh,
}

/// What a click on the menu asks of the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    Logout,
    Quit,
}

/// One kind of options: as they are now, as they were when their window opened (what Cancel goes back to), and whether they changed since
/// the game last took them.
pub(super) struct Group<T> {
    pub(super) now: T,
    pub(super) before: T,
    pub(super) changed: bool,
}

impl<T: Copy + PartialEq> Group<T> {
    /// The game takes the options on its first frame too.
    pub(super) fn new(now: T) -> Group<T> {
        Group { now, before: now, changed: true }
    }

    pub(super) fn set(&mut self, to: T) {
        self.now = to;
        self.changed = true;
    }

    /// The window closes: keep the options (`save`) or go back to what they were.
    pub(super) fn finish(&mut self, keep: bool, save: impl FnOnce(&T)) {
        if keep {
            save(&self.now);
        } else if self.now != self.before {
            self.set(self.before);
        }
    }

    pub(super) fn take(&mut self) -> Option<T> {
        std::mem::take(&mut self.changed).then_some(self.now)
    }
}

pub struct MenuState {
    sound: Group<SoundSettings>,
    video: Group<VideoSettings>,
    interface: Group<InterfaceSettings>,
    /// The slider being dragged, and whether it was the interface scale (taken into use at the release).
    dragging: Option<&'static str>,
    scale_pending: bool,
    action: Option<MenuAction>,
    /// The video modes of the monitor (width, height, Hz), the dropdown that is open and what its rows stand for (the text and two numbers).
    modes: Vec<(u32, u32, u32)>,
    list: Option<Dropdown>,
    rows: Vec<(String, u32, u32)>,
}

impl Default for MenuState {
    fn default() -> Self {
        MenuState { sound: Group::new(SoundSettings::load()), video: Group::new(VideoSettings::load()), interface: Group::new(InterfaceSettings::load()), dragging: None, scale_pending: false, action: None, modes: Vec::new(), list: None, rows: Vec::new() }
    }
}

/// A slider: its value, where it says it changed, and what it runs over (`minValue`, `maxValue`, `valueStep`).
struct Slider<'a> {
    value: &'a mut f32,
    changed: &'a mut bool,
    range: (f32, f32, f32),
}

pub(super) fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate(GAME_MENU)?;
    tree.set_enabled("GameMenuButtonMacros", false);

    tree.instantiate(SOUND)?;
    // Only what the game has: all sound, ambience and music, and the four volumes.
    for i in [4, 6, 7, 8] {
        tree.show(&format!("{SOUND_CHECK}{i}"), false);
    }
    // The three switches stand in a column of one size (the original's second one is smaller and indented, for the speech switch it had).
    for (i, above) in [(2, 1), (5, 2)] {
        tree.set_size(&format!("{SOUND_CHECK}{i}"), Some(32.0), Some(32.0));
        tree.set_point(&format!("{SOUND_CHECK}{i}"), Point::Top, Some(&format!("{SOUND_CHECK}{above}")), Some(Point::Bottom), 0.0, 6.0);
    }
    for (name, key) in [("CheckButton1", "ENABLE_ALL_SOUND"), ("CheckButton2", "ENABLE_AMBIENCE"), ("CheckButton5", "ENABLE_MUSIC"), ("Slider1", "MASTER_VOLUME"), ("Slider2", "SOUND_VOLUME"), ("Slider3", "MUSIC_VOLUME"), ("Slider4", "AMBIENCE_VOLUME")] {
        tree.set_text(&format!("{SOUND}{name}Text"), &tree.string(key));
    }

    tree.instantiate(VIDEO)?;
    // What the game has of its boxes: the display mode, the resolution and refresh rate and the interface scale; how far props and
    // terrain are seen, how finely terrain is filtered, spell detail and weather; gamma; vertical sync.
    for i in [1, 2, 3, 4, 6, 7, 8, 10, 12, 13, 14, 15, 16, 17, 18] {
        tree.show(&format!("OptionsFrameCheckButton{i}"), false);
    }
    for i in [4, 5] {
        tree.show(&format!("OptionsFrameSlider{i}"), false);
    }
    tree.hide_textures_of(VIDEO, "NvidiaLogo");
    tree.set_size(VIDEO, None, Some(500.0));
    tree.set_size("OptionsFrameWorldAppearance", None, Some(100.0));
    tree.set_size("OptionsFrameBrightness", None, Some(55.0));
    tree.set_size("OptionsFrameMiscellaneous", Some(425.0), Some(45.0));
    // The spell detail and the weather take the second row of the box.
    tree.set_point("OptionsFrameSlider8", Point::Top, Some("OptionsFrameSlider2"), Some(Point::Bottom), 0.0, -23.0);
    tree.set_point("OptionsFrameSlider6", Point::TopLeft, Some("OptionsFrameBrightness"), Some(Point::TopLeft), 20.0, -25.0);
    // The pixel shaders' box has the one switch, in the width of the others.
    tree.set_size("OptionsFramePixelShaders", Some(425.0), Some(45.0));
    tree.set_point("OptionsFrameCheckButton11", Point::TopLeft, Some("OptionsFramePixelShaders"), Some(Point::TopLeft), 15.0, -10.0);
    tree.set_point("OptionsFrameMiscellaneous", Point::TopLeft, Some("OptionsFramePixelShaders"), Some(Point::BottomLeft), 0.0, -15.0);
    tree.set_point("OptionsFrameCheckButton5", Point::TopLeft, Some("OptionsFrameMiscellaneous"), Some(Point::TopLeft), 15.0, -10.0);
    for (name, key) in [
        ("DisplayTitle", "DISPLAY"),
        ("WorldAppearanceTitle", "WORLD_APPEARANCE"),
        ("BrightnessTitle", "OPTIONS_BRIGHTNESS"),
        ("MiscellaneousTitle", "MISCELLANEOUS"),
        ("PixelShadersTitle", "PIXEL_SHADERS"),
        ("CheckButton9Text", "USE_UISCALE"),
        ("CheckButton5Text", "VERTICAL_SYNC"),
        ("Slider1Text", "UI_SCALE"),
        ("Slider2Text", "FARCLIP"),
        ("Slider3Text", "ENVIRONMENT_DETAIL"),
        ("Slider6Text", "GAMMA"),
        ("Slider7Text", "ANISOTROPIC"),
        ("Slider8Text", "SPELL_DETAIL"),
        ("Slider9Text", "WEATHER_DETAIL"),
    ] {
        tree.set_text(&format!("{VIDEO}{name}"), &tree.string(key));
    }
    // (The original has no such string: its pixel shaders are the terrain highlights, the glow and the death effect, none of which this game draws.)
    tree.set_text(&format!("{VIDEO}CheckButton11Text"), "Shadows");
    // The dropdowns are as wide as the original makes them (`UIDropDownMenu_SetWidth`): the middle of their picture and a frame round it.
    // The display mode takes the place of the multisample dropdown, and comes first.
    for (name, middle) in [("MultiSample", 110.0), ("Resolution", 80.0), ("Refresh", 80.0)] {
        tree.set_size(&format!("OptionsFrame{name}DropDown"), Some(middle + 50.0), None);
        tree.set_region_size(&format!("OptionsFrame{name}DropDownMiddle"), Some(middle), None);
    }
    tree.set_point("OptionsFrameMultiSampleDropDown", Point::TopLeft, Some("OptionsFrameDisplay"), Some(Point::TopLeft), -8.0, -20.0);
    tree.set_point("OptionsFrameResolutionDropDown", Point::Left, Some("OptionsFrameMultiSampleDropDown"), Some(Point::Right), 0.0, 0.0);
    tree.set_point("OptionsFrameRefreshDropDown", Point::Left, Some("OptionsFrameResolutionDropDown"), Some(Point::Right), 0.0, 0.0);
    tree.set_point("OptionsFrameCheckButton9", Point::TopLeft, Some("OptionsFrameMultiSampleDropDown"), Some(Point::BottomLeft), 17.0, -5.0);
    tree.set_text("OptionsFrameMultiSampleDropDownLabel", "Display Mode");
    // The list the resolution and refresh rate dropdowns open: a row for each choice, above the options windows.
    tree.instantiate(LIST)?;
    tree.set_level(LIST, 20);
    for i in 1..=LIST_MAX {
        tree.set_point(&format!("{LIST}Button{i}"), Point::TopLeft, Some(LIST), Some(Point::TopLeft), 15.0, -15.0 - (i - 1) as f32 * LIST_ROW);
        tree.set_size(&format!("{LIST}Button{i}"), Some(150.0), Some(LIST_ROW));
        // The text stands to the right of the check mark (`UIDropDownMenu_AddButton` does the same for the rows that can be checked).
        tree.set_region_point(&format!("{LIST}Button{i}NormalText"), Point::Left, Some(&format!("{LIST}Button{i}")), Some(Point::Left), 22.0, 0.0);
    }
    // Both of its backdrops, one over the other: one alone lets the window behind it show through.
    tree.set_backdrop_colors("DropDownList1MenuBackdrop", Some([0.0, 0.0, 0.0, 4.0]), None);
    tree.set_backdrop_colors("DropDownList1Backdrop", Some([0.0, 0.0, 0.0, 4.0]), None);

    super::keybinds::setup(tree)?;

    tree.instantiate(INTERFACE)?;
    // The basic page only (no tabs), with what the game has: inverting the mouse; names over creatures and buff durations; how fast the mouse
    // looks round and how far the camera stands.
    for part in ["AdvancedOptions", "BasicOptionsHelp", "UIOptionsFrameTab1", "UIOptionsFrameTab2", "UIOptionsFrameSlider1", "UIOptionsFrameSlider2", "UIOptionsFrameClickCameraDropDown", "UIOptionsFrameCameraDropDown"] {
        tree.show(part, false);
    }
    for i in [5, 6, 12, 14, 15, 16, 20, 21, 22, 23, 24, 26, 27, 31, 42, 43, 45, 61, 62, 66, 67] {
        tree.show(&format!("UIOptionsFrameCheckButton{i}"), false);
    }
    tree.set_size("BasicOptionsGeneral", None, Some(45.0));
    tree.set_size("BasicOptionsDisplay", None, Some(139.0));
    tree.set_size("BasicOptionsCamera", None, Some(65.0));
    tree.set_point("UIOptionsFrameCheckButton1", Point::TopLeft, Some("BasicOptionsGeneral"), Some(Point::TopLeft), 10.0, -8.0);
    tree.set_point("UIOptionsFrameCheckButton30", Point::TopLeft, Some("BasicOptionsDisplay"), Some(Point::TopLeft), 10.0, -8.0);
    tree.set_point("UIOptionsFrameCheckButton39", Point::Top, Some("UIOptionsFrameCheckButton30"), Some(Point::Bottom), 0.0, 0.0);
    tree.set_point("UIOptionsFrameCheckButton3", Point::Top, Some("UIOptionsFrameCheckButton39"), Some(Point::Bottom), 0.0, 0.0);
    tree.set_point("UIOptionsFrameCheckButton2", Point::Top, Some("UIOptionsFrameCheckButton3"), Some(Point::Bottom), 0.0, 0.0);
    // Ragdolls is a child of the Controls box, moved into the Display box: draw Controls last or Display's backdrop veils it.
    tree.set_level("BasicOptionsGeneral", 1);
    // (The original has no such string.)
    tree.set_text("UIOptionsFrameCheckButton3Text", "Ragdolls");
    tree.set_text("UIOptionsFrameCheckButton2Text", "Nodes on the compass");
    tree.set_point("UIOptionsFrameSlider3", Point::TopLeft, Some("BasicOptionsCamera"), Some(Point::TopLeft), 20.0, -28.0);
    tree.set_point("UIOptionsFrameSlider4", Point::Left, Some("UIOptionsFrameSlider3"), Some(Point::Right), 60.0, 0.0);
    // The buttons go under the boxes: the original's are at the bottom of the screen, where the action bar stands.
    tree.set_point("UIOptionsFrameDefaults", Point::TopLeft, Some("BasicOptionsCamera"), Some(Point::BottomLeft), 0.0, -15.0);
    tree.set_point("UIOptionsFrameCancel", Point::TopRight, Some("BasicOptionsCamera"), Some(Point::BottomRight), 0.0, -15.0);
    for (name, key) in [("BasicOptionsGeneralTitle", "CONTROLS_LABEL"), ("BasicOptionsDisplayTitle", "DISPLAY"), ("BasicOptionsCameraTitle", "CAMERA_LABEL"), ("UIOptionsFrameCheckButton1Text", "INVERT_MOUSE"), ("UIOptionsFrameCheckButton30Text", "SHOW_NPC_NAMES"), ("UIOptionsFrameCheckButton39Text", "SHOW_BUFF_DURATION_TEXT"), ("UIOptionsFrameSlider3Text", "MOUSE_LOOK_SPEED"), ("UIOptionsFrameSlider4Text", "MAX_FOLLOW_DIST")] {
        tree.set_text(name, &tree.string(key));
    }
    Ok(())
}

impl Panels {
    pub fn menu_open(&self) -> bool {
        self.tree.is_name_shown(GAME_MENU)
    }

    /// One of the options windows is open.
    pub fn options_open(&self) -> bool {
        self.keys_open() || [SOUND, VIDEO, INTERFACE].iter().any(|f| self.tree.is_name_shown(f))
    }

    /// Esc with nothing else to close: the menu comes up.
    pub fn open_menu(&mut self) {
        self.close_all();
        self.tree.show(GAME_MENU, true);
        self.events.push(Sfx::Named("igMainMenuOpen"));
    }

    /// The menu or its options are what is on top: Esc closes the options (without keeping what was changed) and then the menu.
    pub(super) fn escape_menu(&mut self) -> bool {
        if self.options_open() {
            self.options_done(false);
        } else if self.menu_open() {
            self.close_menu();
        } else {
            return false;
        }
        true
    }

    fn close_menu(&mut self) {
        self.tree.show(GAME_MENU, false);
        self.events.push(Sfx::Named("igMainMenuContinue"));
        self.after_close();
    }

    /// What the menu was asked to do, once.
    pub fn take_menu_action(&mut self) -> Option<MenuAction> {
        self.menu.action.take()
    }

    /// The options, if they changed since the last call.
    pub fn take_sound(&mut self) -> Option<SoundSettings> {
        self.menu.sound.take()
    }

    pub fn take_video(&mut self) -> Option<VideoSettings> {
        self.menu.video.take()
    }

    pub fn take_interface(&mut self) -> Option<InterfaceSettings> {
        self.menu.interface.take()
    }

    /// Whether the veins and herbs near show on the compass.
    pub fn compass_nodes(&self) -> bool {
        self.menu.interface.now.compass_nodes
    }

    /// Whether the buffs show how long they last.
    pub(super) fn show_durations(&self) -> bool {
        self.menu.interface.now.buff_durations
    }

    /// Leave the options window for the menu, keeping what was set or going back to what it was.
    pub(super) fn options_done(&mut self, keep: bool) {
        let keys = self.keys_open();
        let m = &mut self.menu;
        if keys {
            self.keys_done(keep);
        } else if self.tree.is_name_shown(SOUND) {
            m.sound.finish(keep, SoundSettings::save);
        } else if self.tree.is_name_shown(VIDEO) {
            m.video.finish(keep, VideoSettings::save);
        } else {
            m.interface.finish(keep, InterfaceSettings::save);
        }
        for frame in [SOUND, VIDEO, INTERFACE, LIST] {
            self.tree.show(frame, false);
        }
        self.menu.list = None;
        self.tree.show(GAME_MENU, true);
        self.after_close();
    }

    fn open_options(&mut self, frame: &str) {
        let m = &mut self.menu;
        (m.sound.before, m.video.before, m.interface.before) = (m.sound.now, m.video.now, m.interface.now);
        self.tree.show(GAME_MENU, false);
        self.tree.show(frame, true);
        self.events.push(Sfx::Named("igMainMenuOption"));
        self.options_refresh();
    }

    /// The switch a check button of the options stands for, and where its group says it changed.
    fn flag_mut(&mut self, name: &str) -> Option<(&mut bool, &mut bool)> {
        let m = &mut self.menu;
        Some(match name {
            "SoundOptionsFrameCheckButton1" => (&mut m.sound.now.all, &mut m.sound.changed),
            "SoundOptionsFrameCheckButton2" => (&mut m.sound.now.ambience, &mut m.sound.changed),
            "SoundOptionsFrameCheckButton5" => (&mut m.sound.now.music, &mut m.sound.changed),
            "OptionsFrameCheckButton9" => (&mut m.video.now.use_ui_scale, &mut m.video.changed),
            "OptionsFrameCheckButton11" => (&mut m.video.now.shadows, &mut m.video.changed),
            "OptionsFrameCheckButton5" => (&mut m.video.now.vsync, &mut m.video.changed),
            "UIOptionsFrameCheckButton1" => (&mut m.interface.now.invert_mouse, &mut m.interface.changed),
            "UIOptionsFrameCheckButton30" => (&mut m.interface.now.npc_names, &mut m.interface.changed),
            "UIOptionsFrameCheckButton39" => (&mut m.interface.now.buff_durations, &mut m.interface.changed),
            "UIOptionsFrameCheckButton3" => (&mut m.interface.now.ragdolls, &mut m.interface.changed),
            "UIOptionsFrameCheckButton2" => (&mut m.interface.now.compass_nodes, &mut m.interface.changed),
            _ => return None,
        })
    }

    fn slider_mut(&mut self, name: &str) -> Option<Slider<'_>> {
        let m = &mut self.menu;
        let (value, changed, range) = match name {
            "SoundOptionsFrameSlider1" => (&mut m.sound.now.master, &mut m.sound.changed, (0.0, 1.0, 0.1)),
            "SoundOptionsFrameSlider2" => (&mut m.sound.now.sound, &mut m.sound.changed, (0.0, 1.0, 0.1)),
            "SoundOptionsFrameSlider3" => (&mut m.sound.now.music_volume, &mut m.sound.changed, (0.0, 1.0, 0.1)),
            "SoundOptionsFrameSlider4" => (&mut m.sound.now.ambience_volume, &mut m.sound.changed, (0.0, 1.0, 0.1)),
            "OptionsFrameSlider1" => (&mut m.video.now.ui_scale, &mut m.video.changed, (0.64, 1.0, 0.01)),
            "OptionsFrameSlider2" => (&mut m.video.now.terrain, &mut m.video.changed, (0.0, 1.0, 0.1)),
            "OptionsFrameSlider3" => (&mut m.video.now.env_detail, &mut m.video.changed, (0.0, 2.0, 1.0)),
            "OptionsFrameSlider6" => (&mut m.video.now.gamma, &mut m.video.changed, (-0.5, 0.5, 0.1)),
            "OptionsFrameSlider8" => (&mut m.video.now.spell_detail, &mut m.video.changed, (0.0, 2.0, 1.0)),
            "OptionsFrameSlider7" => (&mut m.video.now.anisotropy, &mut m.video.changed, (1.0, 5.0, 1.0)),
            "OptionsFrameSlider9" => (&mut m.video.now.weather, &mut m.video.changed, (0.0, 3.0, 1.0)),
            "UIOptionsFrameSlider3" => (&mut m.interface.now.look_speed, &mut m.interface.changed, (90.0, 270.0, 10.0)),
            "UIOptionsFrameSlider4" => (&mut m.interface.now.follow, &mut m.interface.changed, (1.0, 2.0, 0.1)),
            _ => return None,
        };
        Some(Slider { value, changed, range })
    }

    /// A click on a button of these windows. False when it was none of them.
    pub(crate) fn menu_click(&mut self, name: &str) -> bool {
        if self.dropdown_click(name) || self.keys_click(name) {
            return true;
        }
        let sfx = match name {
            "GameMenuButtonOptions" | "GameMenuButtonSoundOptions" | "GameMenuButtonUIOptions" => {
                self.open_options(match name {
                    "GameMenuButtonOptions" => VIDEO,
                    "GameMenuButtonSoundOptions" => SOUND,
                    _ => INTERFACE,
                });
                return true;
            }
            "GameMenuButtonKeybindings" => {
                self.open_keys();
                return true;
            }
            "GameMenuButtonContinue" => {
                self.close_menu();
                return true;
            }
            "GameMenuButtonLogout" => {
                self.menu.action = Some(MenuAction::Logout);
                "igMainMenuLogout"
            }
            "GameMenuButtonQuit" => {
                self.menu.action = Some(MenuAction::Quit);
                "igMainMenuQuit"
            }
            "SoundOptionsFrameOkay" | "OptionsFrameOkay" | "UIOptionsFrameOkay" => {
                self.options_done(true);
                "gsTitleOptionOK"
            }
            "SoundOptionsFrameCancel" | "OptionsFrameCancel" | "UIOptionsFrameCancel" => {
                self.options_done(false);
                "gsTitleOptionExit"
            }
            "SoundOptionsFrameDefaults" | "OptionsFrameDefaults" | "UIOptionsFrameDefaults" => {
                if self.tree.is_name_shown(SOUND) {
                    self.menu.sound.set(Default::default());
                } else if self.tree.is_name_shown(VIDEO) {
                    self.menu.video.set(Default::default());
                } else {
                    self.menu.interface.set(Default::default());
                }
                self.options_refresh();
                "igMainMenuOptionCheckBoxOn"
            }
            _ => {
                let Some((flag, changed)) = self.flag_mut(name) else { return false };
                *flag = !*flag;
                *changed = true;
                let sfx = if *flag { "igMainMenuOptionCheckBoxOn" } else { "igMainMenuOptionCheckBoxOff" };
                self.options_refresh();
                sfx
            }
        };
        self.events.push(Sfx::Named(sfx));
        true
    }

    /// Show the options: the check buttons (ambience and music need all sound to be on) and the thumbs of the sliders.
    fn options_refresh(&mut self) {
        let (s, v, i) = (self.menu.sound.now, self.menu.video.now, self.menu.interface.now);
        for slider in SLIDERS {
            self.place_thumb(slider);
        }
        let dependent = |name: &str| name == "SoundOptionsFrameCheckButton2" || name == "SoundOptionsFrameCheckButton5";
        let flags = [
            ("SoundOptionsFrameCheckButton1", s.all),
            ("SoundOptionsFrameCheckButton2", s.ambience),
            ("SoundOptionsFrameCheckButton5", s.music),
            ("OptionsFrameCheckButton9", v.use_ui_scale),
            ("OptionsFrameCheckButton11", v.shadows),
            ("OptionsFrameCheckButton5", v.vsync),
            ("UIOptionsFrameCheckButton1", i.invert_mouse),
            ("UIOptionsFrameCheckButton30", i.npc_names),
            ("UIOptionsFrameCheckButton39", i.buff_durations),
            ("UIOptionsFrameCheckButton3", i.ragdolls),
            ("UIOptionsFrameCheckButton2", i.compass_nodes),
        ];
        for (name, flag) in flags {
            self.tree.set_checked(name, flag);
            self.tree.set_enabled(name, s.all || !dependent(name));
        }
        let default = "Default".to_string();
        let resolution = if v.resolution == (0, 0) { default.clone() } else { format!("{}x{}", v.resolution.0, v.resolution.1) };
        let refresh = if v.refresh == 0 { default } else { format!("{} Hz", v.refresh) };
        self.tree.set_text("OptionsFrameResolutionDropDownText", &resolution);
        let mode = DisplayMode::ALL.iter().find(|m| m.0 == v.mode).map_or("", |m| m.1);
        self.tree.set_text("OptionsFrameMultiSampleDropDownText", mode);
        self.tree.set_text("OptionsFrameRefreshDropDownText", &refresh);
    }

    /// The video modes the monitor has, for the resolution and refresh rate dropdowns.
    pub fn set_video_modes(&mut self, modes: Vec<(u32, u32, u32)>) {
        self.menu.modes = modes;
    }

    /// A click on a dropdown or on a row of its list; any other click closes the list. True when it was one of those.
    fn dropdown_click(&mut self, name: &str) -> bool {
        let open = self.menu.list.take();
        self.tree.show(LIST, false);
        if let Some(row) = name.strip_prefix("DropDownList1Button").and_then(|n| n.parse::<usize>().ok()) {
            if let (Some(kind), Some(&(_, a, b))) = (open, self.menu.rows.get(row.wrapping_sub(1))) {
                let video = &mut self.menu.video;
                match kind {
                    Dropdown::Resolution => {
                        video.now.resolution = (a, b);
                        // A refresh rate that the new size does not have goes back to the monitor's own.
                        let (m, r) = (&self.menu.modes, video.now.refresh);
                        if r != 0 && (a, b) != (0, 0) && !m.iter().any(|&(w, h, hz)| (w, h) == (a, b) && hz == r) {
                            video.now.refresh = 0;
                        }
                    }
                    Dropdown::Refresh => video.now.refresh = a,
                    Dropdown::Mode => video.now.mode = DisplayMode::ALL[a as usize].0,
                }
                video.changed = true;
                self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
                self.options_refresh();
            }
            return true;
        }
        let kind = if name.starts_with("OptionsFrameMultiSampleDropDown") {
            Dropdown::Mode
        } else if name.starts_with("OptionsFrameResolutionDropDown") {
            Dropdown::Resolution
        } else if name.starts_with("OptionsFrameRefreshDropDown") {
            Dropdown::Refresh
        } else {
            return false;
        };
        // Clicking the dropdown whose list is open closes it.
        if open != Some(kind) {
            self.open_list(kind);
        }
        true
    }

    fn open_list(&mut self, kind: Dropdown) {
        let (modes, video) = (&self.menu.modes, self.menu.video.now);
        let mut rows = vec![("Default".to_string(), 0, 0)];
        let selected = match kind {
            Dropdown::Mode => {
                rows = DisplayMode::ALL.iter().enumerate().map(|(i, m)| (m.1.to_string(), i as u32, 0)).collect();
                DisplayMode::ALL.iter().position(|m| m.0 == video.mode)
            }
            Dropdown::Resolution => {
                for &(w, h, _) in modes {
                    if !rows.iter().any(|r| (r.1, r.2) == (w, h)) {
                        rows.push((format!("{w}x{h}"), w, h));
                    }
                }
                rows.iter().position(|r| (r.1, r.2) == video.resolution)
            }
            Dropdown::Refresh => {
                for &(w, h, hz) in modes {
                    if (video.resolution == (0, 0) || (w, h) == video.resolution) && !rows.iter().any(|r| r.1 == hz) {
                        rows.push((format!("{hz} Hz"), hz, 0));
                    }
                }
                rows.iter().position(|r| r.1 == video.refresh)
            }
        };
        rows.truncate(LIST_MAX);
        let owner = match kind {
            Dropdown::Mode => "OptionsFrameMultiSampleDropDown",
            Dropdown::Resolution => "OptionsFrameResolutionDropDown",
            Dropdown::Refresh => "OptionsFrameRefreshDropDown",
        };
        self.tree.set_size(LIST, Some(180.0), Some(30.0 + rows.len() as f32 * LIST_ROW));
        self.tree.set_point(LIST, Point::TopLeft, Some(owner), Some(Point::BottomLeft), 10.0, 5.0);
        for i in 1..=LIST_MAX {
            let button = format!("{LIST}Button{i}");
            self.tree.show(&button, i <= rows.len());
            if let Some(row) = rows.get(i - 1) {
                self.tree.set_text(&format!("{button}NormalText"), &row.0);
                self.tree.set_region_hidden(&format!("{button}Check"), selected != Some(i - 1));
            }
        }
        (self.menu.list, self.menu.rows) = (Some(kind), rows);
        self.tree.show(LIST, true);
        self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
    }

    /// Move a slider's thumb to where its value is.
    fn place_thumb(&mut self, name: &str) {
        let Some(slider) = self.slider_mut(name) else { return };
        let (min, max, _) = slider.range;
        let t = (*slider.value - min) / (max - min);
        self.tree.set_region_point(&format!("{name}Thumb"), Point::Center, Some(name), Some(Point::Left), t * SLIDER_WIDTH, 0.0);
    }

    /// A press on a slider: its thumb goes to the pointer, and follows it while the button is held.
    pub(super) fn slider_press(&mut self, name: &str, pos: (f32, f32), screen: (u32, u32)) {
        self.menu.dragging = SLIDERS.into_iter().find(|s| *s == name);
        self.slider_drag(pos, screen);
    }

    pub(super) fn slider_release(&mut self) {
        self.menu.dragging = None;
        if std::mem::take(&mut self.menu.scale_pending) {
            self.menu.video.changed = true;
        }
    }

    pub(super) fn slider_drag(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        let Some(name) = self.menu.dragging else { return };
        let Some(r) = self.tree.rect_of(name, screen) else { return };
        let at = ((pos.0 / (screen.1 as f32 / ui_height()) - r.x) / r.w).clamp(0.0, 1.0);
        let Some(slider) = self.slider_mut(name) else { return };
        let (min, max, step) = slider.range;
        let value = (min + ((at * (max - min)) / step).round() * step).clamp(min, max);
        if (*slider.value - value).abs() > 1e-4 {
            *slider.value = value;
            if name == SCALE_SLIDER {
                self.menu.scale_pending = true;
            } else {
                *slider.changed = true;
            }
            self.place_thumb(name);
        }
    }
}
