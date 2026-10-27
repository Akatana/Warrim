//! Interactive window: input, game loop, presentation.

use crate::chest::{Chests, OPEN_RANGE};
use crate::cursor::{Cursors, Hover, Kind};
use crate::physics::Bodies;
use crate::lifts::Solid;
use crate::pickups::Pickups;
use crate::loading::{Loading, Stage};
use crate::lockpick::{Event, Lockpick};
use crate::camera::{Camera, Controller, Input, Mode, EYE_HEIGHT};
use crate::coords::{render_to_wow, wow_to_render};
use crate::actionbar::ActionBar;
use crate::audio::{Audio, Cry, Sfx};
use crate::combattext::CombatText;
use crate::character::Appearance;
use crate::console::Command;
use crate::content;
use crate::skills;
use crate::creature::Creatures;
use crate::panels::Panels;
use crate::models::ModelCache;
use glam::{Mat4, Vec3};
use crate::player::{Motion, Player, View};
use crate::renderer::{Atmosphere, Gpu, Renderer};
use crate::glue::{Glue, Key as GlueKey, Outcome};
use crate::save;
use crate::sky::Sky;
use crate::world::World;
use crate::Args;
use anyhow::{Context, Result};
use mpq::Mount;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

mod commands;
mod environment;
mod events;
mod frame;
mod input;
mod crafting;
mod fishing;
mod interact;
mod portals;
mod session;
mod setup;
mod windows;

use windows::{open_gpu, present_loading, weather_at_start, CinemaWindow, GlueWindow};

/// Seconds between two automatic saves.
const SAVE_SECONDS: u64 = 30;
/// The name of a character played without the list (`--no-save`, `--save`).
const DEFAULT_NAME: &str = "Adventurer";
/// Loot window numbers from this one up are the chests' (the creatures' are below).
const CHEST_LOOT: usize = 1_000_000;
/// Loot window numbers from this one up are the nodes' (veins and herbs).
const NODE_LOOT: usize = 2_000_000;
/// The loot window of what a line brought in (nothing is left on the water).
const FISH_LOOT: usize = 3_000_000;

/// How fast the editor's camera flies at first (yards a second; the wheel changes it).
const FREE_SPEED: f32 = 25.0;

/// A gathering being cast: the node, the seconds left and the models the original shows in the hands meanwhile.
struct Gather {
    job: Job,
    left: f32,
    glows: Vec<(usize, u32)>,
}

/// What is being worked at: a node to gather, or a recipe to make (a place in the content) with how many times more after this one.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Job {
    Node(usize),
    Make { recipe: usize, more: u32 },
    /// The hide of the corpse of creature `n`.
    Skin(usize),
    /// The line is out.
    Fish,
    /// A lure or a stone (the item) goes on the weapon.
    Lure(usize),
}

/// Work that begins when the character has drawn the weapons: the line to cast to a spot, a lure to put on.
#[derive(Clone, Copy)]
enum Later {
    Fish(Vec3),
    Lure(usize),
}

/// The bobber of a line: its model, where it lies, seconds to the bite, seconds the bite lasts and its own clock.
struct Bobber {
    entity: usize,
    /// The ribbon of the line from the pole to it.
    line: usize,
    pos: Vec3,
    bite_in: f32,
    bite_left: Option<f32>,
    time: f32,
}

/// The editor's camera: it flies without bumping into anything, and the player stays where he was (the creatures cannot hurt him
/// meanwhile).
struct FreeCam {
    camera: Camera,
    controller: Controller,
}
/// How many times heavier a sneaking player's blow on an unsuspecting creature is.
const SNEAK_ATTACK: f32 = 3.0;
/// How close a corpse must be to loot it (yards).
const LOOT_RANGE: f32 = 5.0;
/// How close a friendly creature must be to talk to it (yards).
const TALK_RANGE: f32 = 5.0;
/// How far (and how much higher or lower) a bed's origin may be from the feet for it to be slept in.
const BED_RANGE: f32 = 4.0;
const BED_HEIGHT: f32 = 3.0;
/// How near a book, a letter or a plaque has to be to be read, yards.
const READ_RANGE: f32 = 4.0;
/// How far the player may walk off before the loot or gossip window closes (yards).
const LOOT_LEAVE: f32 = 8.0;

pub fn run(args: Args, mount: Arc<Mount>) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { args, mount, glue: None, cinema: None, state: None, error: None };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct App {
    args: Args,
    mount: Arc<Mount>,
    /// The list of characters, until one is chosen and the game takes its place.
    glue: Option<GlueWindow>,
    /// The introduction of the race of a character just made, until it ends.
    cinema: Option<CinemaWindow>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}


/// The places of the props that are beds in `map`.
fn bed_spots(props: &[content::Prop], map: &str) -> Vec<Vec3> {
    props.iter().filter(|p| p.sleep && p.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map))).map(|p| crate::coords::wow_to_render([p.at.0, p.at.1, p.at.2])).collect()
}

/// Where the things to read are in `map` (render space), with the first page of each and what its window is headed with.
fn book_spots(props: &[content::Prop], map: &str) -> Vec<(Vec3, u32, String)> {
    props.iter().filter(|p| p.page != 0 && p.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map))).map(|p| (crate::coords::wow_to_render([p.at.0, p.at.1, p.at.2]), p.page, p.title.clone().unwrap_or_default())).collect()
}

/// The nearest thing to read within reach of `feet`: the place in `books`.
fn book_near(books: &[(Vec3, u32, String)], feet: Vec3) -> Option<usize> {
    let reach = |b: &Vec3| ((b.x - feet.x).hypot(b.z - feet.z), (b.y - feet.y).abs());
    books.iter().enumerate().filter(|(_, (b, ..))| reach(b).0 <= READ_RANGE && reach(b).1 < BED_HEIGHT).min_by(|a, b| reach(&a.1.0).0.total_cmp(&reach(&b.1.0).0)).map(|(i, _)| i)
}

/// Where the stations of the trades (anvils, forges, cooking fires) stand in `map`.
fn station_spots(props: &[content::Prop], map: &str) -> Vec<(u32, Vec3)> {
    props.iter().filter(|p| p.station != 0 && p.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map))).map(|p| (p.station, crate::coords::wow_to_render([p.at.0, p.at.1, p.at.2]))).collect()
}

/// How near a station has to be for the trade to be done at it, yards (the original's objects say 5 to 10).
const STATION_RANGE: f32 = 10.0;

/// Whether a station of this kind (a row of `SpellFocusObject.dbc`) is within reach of `feet`.
fn near_station(stations: &[(u32, Vec3)], feet: Vec3, kind: u32) -> bool {
    stations.iter().any(|&(k, p)| k == kind && (p.x - feet.x).hypot(p.z - feet.z) <= STATION_RANGE && (p.y - feet.y).abs() < 6.0)
}

/// Whether a bed is within reach of `feet`.
fn near_bed(beds: &[Vec3], world: &World, feet: Vec3) -> bool {
    world.bed_near(feet, BED_RANGE, BED_HEIGHT) || beds.iter().any(|b| (b.x - feet.x).hypot(b.z - feet.z) <= BED_RANGE && (b.y - feet.y).abs() < BED_HEIGHT)
}

/// The props of the content that are drawn apart from the map: those picked up and those with physics.
fn pickup_props() -> Vec<content::Prop> {
    content::get().props.iter().filter(|p| p.pickup.is_some() || p.physics).cloned().collect()
}

/// The spawns that stand in `map` (those the editor put down have none and stand in every map).
fn map_spawns(map: &str) -> impl Iterator<Item = &'static content::Spawn> + '_ {
    content::get().spawns.iter().filter(move |s| s.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map)))
}

/// The respawn time and the patrol of each spawn of `map`, in the order of `spawn_list`.
fn spawn_routes(map: &str) -> Vec<(Option<f32>, Vec<content::Waypoint>)> {
    map_spawns(map).map(|s| (s.respawn, s.path.clone())).collect()
}

/// The creatures of the content that stand in `map`: (kind, feet position in render space, heading in radians, yards it strolls).
fn spawn_list(map: &str) -> Vec<(usize, Vec3, f32, f32)> {
    map_spawns(map).map(|s| (content::creature_index(&s.creature), wow_to_render([s.at.0, s.at.1, s.height.unwrap_or(0.0)]), s.facing.to_radians(), s.wander)).collect()
}

struct State {
    /// Who is played: the name and the look (race, sex, hair...), which the save keeps.
    name: String,
    look: Appearance,
    /// The loading screen, until the terrain around the character has streamed in.
    loading: Option<Loading>,
    /// The loading screen put away once the terrain is in, for the next trip.
    spare_loading: Option<Loading>,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    renderer: Renderer,
    sky: Option<Sky>,
    player: Option<Player>,
    audio: Option<Audio>,
    actionbar: Option<ActionBar>,
    /// The pet bar, drawn while the player has a pet.
    petbar: Option<crate::petbar::PetBar>,
    wolves: Option<Creatures>,
    chests: Option<Chests>,
    /// The props that quests send the player to pick up, and the map they are in.
    pickups: Option<Pickups>,
    /// The lifts of the map, which carry the player.
    lifts: Option<crate::lifts::Lifts>,
    /// The loose props with physics, pushed and carried by the player.
    bodies: Option<Bodies>,
    /// The props that are picked up or have physics (the world's props change in the editor), and whether `pickups` and `bodies` are to be made again from them.
    pickup_props: Vec<content::Prop>,
    pickups_stale: bool,
    /// The veins and herbs of the map, and the one being gathered (its place and the seconds left of the cast).
    nodes: Option<crate::nodes::Nodes>,
    gather: Option<Gather>,
    /// The bobber of the line that is out.
    bobber: Option<Bobber>,
    /// What waits for the hand to reach the weapons.
    later: Option<Later>,
    /// Where the beds are (render space): the props flagged `sleep`.
    beds: Vec<Vec3>,
    /// Where the things to read are (render space), the first page of each and its title: the props with a `page`.
    books: Vec<(Vec3, u32, String)>,
    /// Where the stations of the trades are (render space): the props with a `station`, with its kind.
    stations: Vec<(u32, Vec3)>,
    /// A lootable corpse that can also be dragged, and when the interact key went down on it: a tap loots it, holding drags it.
    pending_loot: Option<(usize, Instant)>,
    map: String,
    /// The map the game starts on and the dead come back to, and the portal the character stands in (it takes him on the way in, not while he stays).
    home: String,
    in_portal: Option<u32>,
    /// The loading picture (a texture of the interface) of each map gone to, by lower-case name, so that a trip back does not make it again.
    screens: std::collections::HashMap<String, usize>,
    /// The chest being picked and the lock, while the lockpicking is on; whether the lock is being turned; seconds into it.
    lock: Option<(usize, Lockpick)>,
    lock_turn: bool,
    lock_time: f32,
    lock_art: Option<crate::lockpick::LockArt>,
    /// The rings that rise when a level is gained.
    effects: crate::effects::Effects,
    /// The editor (F1), and whether it asked for the game to start again with what it saved.
    editor: Option<crate::editor::Editor>,
    /// The developer console (F2), and whether god mode (`tgm`) is on.
    console: Option<crate::console::Console>,
    god: bool,
    /// The free camera (`tfc`) is out, as it is while the editor is open.
    detached: bool,
    /// The eyes of a spell are out of the character: the seconds left and how far from him they may go.
    scout: Option<(f32, f32)>,
    restart: bool,
    /// The restart goes back to the list of characters, not into the game.
    logout: bool,
    /// The game menu's Exit Game was clicked.
    quit: bool,
    /// The present mode changed: the surface is configured again after the frame.
    reconfigure: bool,
    /// The names over creatures show (the interface options).
    names: bool,
    /// Whether dying creatures fall as ragdolls (the interface options).
    ragdolls: bool,
    /// The editor asked for the content it saved to be taken into use (and when to ask it by itself: `--apply-after`).
    apply: bool,
    apply_after: Option<f32>,
    mount: Arc<Mount>,
    /// The level up effect while it plays, and whether the character moved this frame (a cast fails when he does).
    levelfx: Option<usize>,
    /// The sound entry of the original's Stealth, played when a sneak begins.
    stealth_sound: Option<u32>,
    /// What the keys do (the key bindings window changes it, `take_keys`).
    binds: crate::keys::Binds,
    moving: bool,
    caster: crate::casting::Spellcaster,
    /// What the spells of creatures do to the player, and the auras that have hold of him.
    foes: crate::foes::Foes,
    fires: crate::fires::Fires,
    /// A spell the bar asked to cast, for the frame to do.
    cast_request: Option<usize>,
    /// A corpse is close enough to loot: tell the player how.
    /// What the interact key would do now ("Press E to loot"), if anything.
    hint: Option<&'static str>,
    /// The pointer's pictures, and what the pointer is over in the world.
    cursors: Option<Cursors>,
    hover: Option<Hover>,
    pointer: Kind,
    combat_text: Option<CombatText>,
    panels: Option<Panels>,
    /// In combat (a wolf is after the player, and for 5 seconds after), and seconds since the last wolf was.
    in_combat: bool,
    quiet: f32,
    low_health: bool,
    /// Where the player appears at the start and after dying (x, z in render space).
    spawn: (f32, f32),
    /// The action bar slot the left mouse button is holding down.
    mouse_slot: Option<usize>,
    /// The mouse pointer in pixels while it is free (not grabbed for looking around).
    cursor: Option<(f32, f32)>,
    /// Eye position of the previous frame, for the player's speed.
    last_eye: Vec3,
    /// Hour of the day (0..24); fixed when `--time` was given.
    hour: f32,
    fixed_time: bool,
    weather: crate::weather::Weather,
    /// Whether the player is inside a building and what weather the zone has, looked up now and then.
    indoors: bool,
    zone: Option<u32>,
    weather_look: f32,
    /// What is heard from places (streams, waterfalls, fountains), looked for four times a second, and whether the feet are in water.
    places: Vec<(u64, u32, Vec3)>,
    wading: bool,
    /// The ripples the character makes in water, what draws them, how deep the water was last frame, how fast the character goes, and whether it took a
    /// step since the spray was last looked for.
    ripples: crate::ripples::Ripples,
    ripple_draw: Option<crate::ripples::RippleDraw>,
    /// What draws the circle under the selected unit.
    selection_draw: Option<crate::selection::SelectionDraw>,
    water_depth: f32,
    speed: f32,
    stepped: bool,
    sound_look: f32,
    world: World,
    camera: Camera,
    /// How the last frame was seen (the player's camera may stand behind the player): what the editor puts the pointer into the world with.
    eye: Option<crate::propedit::Eye>,
    /// The editor's camera while it is open: it flies free and the player stands where he was.
    free: Option<FreeCam>,
    /// A mouse button is held to look round with the editor's camera (the mouse is held by the window), or to turn the model that is put down.
    looking: bool,
    rotating: bool,
    /// Ctrl is held (Ctrl + S saves in the editor).
    ctrl: bool,
    controller: Controller,
    input: Input,
    /// Still waiting for terrain under the spawn point to stream in.
    snap_to_ground: Option<f32>,
    /// Feet height from the loaded save: the first snap lands on the floor at that height (a bridge, a building), not the terrain.
    saved_feet: Option<f32>,
    grabbed: bool,
    start: Instant,
    last_frame: Instant,
    /// Where the game is saved (nowhere with `--no-save`) and when it was last.
    save_path: Option<std::path::PathBuf>,
    last_save: Instant,
    /// Seconds to the next look at whether a rider has gone indoors.
    indoor_check: f32,
    title_timer: Instant,
    frames: u32,
    /// Time spent in `Renderer::render` since the title was last updated.
    render_time: Duration,
    /// What the creatures (their steps, poses and models) took since the last report.
    creature_time: Duration,
    /// The area and the room the camera is in, looked up ten times a second (the sound and the places found go by them), and the seconds to the next look.
    area_now: Option<u32>,
    room_now: Option<crate::world::Room>,
    place_look: f32,
}

impl State {
    /// The first point of the terrain or of an object that the pointer (pixels) is over.
    fn cursor_hit(&self, pos: (f32, f32)) -> Option<Vec3> {
        let (origin, dir) = self.eye?.ray(glam::Vec2::new(pos.0, pos.1));
        self.world.ray_hit(origin, origin + dir * 400.0)
    }

    fn set_grab(&mut self, grab: bool) {
        // The mouse stays free while the editor is open (but for a moment, to look round).
        let grab = grab && (!self.editor.as_ref().is_some_and(|e| e.open) || self.looking) && !self.console.as_ref().is_some_and(|c| c.open);
        if !grab {
            self.looking = false;
        }
        if grab {
            let ok = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            self.grabbed = ok;
            self.window.set_cursor_visible(!ok);
            if ok {
                self.cursor = None;
                if let Some(bar) = &mut self.actionbar {
                    bar.set_cursor(None);
                }
            }
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.window.set_cursor_visible(self.cursors.is_none());
            self.grabbed = false;
        }
    }

    /// Hold the mouse to look round with the editor's camera (or let it go).
    fn editor_look(&mut self, on: bool) {
        if on != self.looking && (!on || self.editor.as_ref().is_some_and(|e| e.open)) {
            self.looking = on;
            self.set_grab(on);
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.gpu.device, &self.config);
        self.camera.aspect = w as f32 / h as f32;
    }
}

impl App {
    /// Open the game in the window, with the character named in the arguments.
    fn begin(&mut self, window: Arc<Window>) -> Result<()> {
        self.state = Some(State::new(window, &self.args, self.mount.clone())?);
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() || self.glue.is_some() || self.cinema.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("rsworld")
            .with_inner_size(LogicalSize::new(1280.0, 720.0));
        // The list of characters comes first, unless the character was named (or a save file, or no saving at all).
        let list = self.args.intro.is_none() && self.args.character.is_none() && self.args.save && self.args.save_path.is_none() && !self.args.new_game;
        let result = event_loop.create_window(attrs).map_err(anyhow::Error::from).map(Arc::new).and_then(|w| {
            if let Some(race) = self.args.intro {
                self.cinema = Some(CinemaWindow::new(w, self.mount.clone(), race, &self.args)?);
                Ok(())
            } else if list {
                self.glue = Some(GlueWindow::new(w, self.mount.clone(), None, self.args.audio)?);
                Ok(())
            } else {
                self.begin(w)
            }
        });
        if let Err(e) = result {
            self.error = Some(e);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(glue) = &mut self.glue {
            if matches!(event, WindowEvent::CloseRequested) {
                return event_loop.exit();
            }
            match glue.event(&event) {
                None => {}
                Some(Outcome::Quit) => event_loop.exit(),
                Some(Outcome::Play { name, look, new }) => {
                    // The character list lets go of the window; the game takes it, and the save of this character is its own file.
                    let Some(GlueWindow { window, .. }) = self.glue.take() else { return };
                    self.args.look = look;
                    self.args.character = Some(name);
                    // A new character first sees the introduction of its race (skipped when it cannot be shown).
                    let intro = if new { CinemaWindow::new(window.clone(), self.mount.clone(), look.race, &self.args).map_err(|e| log::warn!("no introduction: {e:#}")).ok() } else { None };
                    let result = match intro {
                        Some(cinema) => {
                            self.cinema = Some(cinema);
                            Ok(())
                        }
                        None => self.begin(window),
                    };
                    if let Err(e) = result {
                        self.error = Some(e);
                        event_loop.exit();
                    }
                }
            }
            return;
        }
        if let Some(cinema) = &mut self.cinema {
            if matches!(event, WindowEvent::CloseRequested) {
                return event_loop.exit();
            }
            if cinema.event(&event) {
                let Some(CinemaWindow { window, .. }) = self.cinema.take() else { return };
                if self.args.intro.is_some() {
                    return event_loop.exit();
                }
                if let Err(e) = self.begin(window) {
                    self.error = Some(e);
                    event_loop.exit();
                }
            }
            return;
        }
        if let Some(state) = self.state.as_mut() {
            state.event(event_loop, event);
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (Some(state), DeviceEvent::MouseMotion { delta }) = (self.state.as_mut(), event) {
            if state.grabbed {
                match &mut state.lock {
                    // The mouse turns the pick, not the head.
                    Some((_, lock)) => lock.rotate(delta.0 as f32),
                    None => match &mut state.free {
                        Some(free) => free.controller.look(&mut free.camera, delta.0 as f32, delta.1 as f32),
                        None => state.controller.look(&mut state.camera, delta.0 as f32, delta.1 as f32),
                    },
                }
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut self.state {
            state.save_game();
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(glue) = &self.glue {
            glue.window.request_redraw();
        }
        if let Some(cinema) = &self.cinema {
            cinema.window.request_redraw();
        }
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}
