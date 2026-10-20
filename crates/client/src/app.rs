//! Interactive window: input, game loop, presentation.

use crate::chest::{Chests, OPEN_RANGE};
use crate::cursor::{Cursors, Hover, Kind};
use crate::pickups::Pickups;
use crate::loading::{Loading, Stage};
use crate::lockpick::{Event, Lockpick};
use crate::camera::{Camera, Controller, Input, Mode, EYE_HEIGHT};
use crate::coords::{render_to_wow, wow_to_render};
use crate::actionbar::{slot_of_key, ActionBar};
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

/// How close a corpse must be to loot it, and how far the player may walk off before the window closes (yards).
/// Seconds between two automatic saves.
const SAVE_SECONDS: u64 = 30;
/// The name of a character played without the list (`--no-save`, `--save`).
const DEFAULT_NAME: &str = "Adventurer";
/// Loot window numbers from this one up are the chests' (the creatures' are below).
const CHEST_LOOT: usize = 1_000_000;

/// How fast the editor's camera flies at first (yards a second; the wheel changes it).
const FREE_SPEED: f32 = 25.0;

/// The editor's camera: it flies without bumping into anything, and the player stays where he was (the creatures cannot hurt him
/// meanwhile).
struct FreeCam {
    camera: Camera,
    controller: Controller,
}
/// How many times heavier a sneaking player's blow on an unsuspecting creature is.
const SNEAK_ATTACK: f32 = 3.0;
const LOOT_RANGE: f32 = 5.0;
/// How close a friendly creature must be to talk to it (yards).
const TALK_RANGE: f32 = 5.0;
/// How far (and how much higher or lower) a bed's origin may be from the feet for it to be slept in.
const BED_RANGE: f32 = 4.0;
const BED_HEIGHT: f32 = 3.0;
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

/// The window's surface and the GPU that draws to it, in gamma space like the original (a surface that is not sRGB is preferred).
fn open_gpu(window: &Arc<Window>) -> Result<(wgpu::Surface<'static>, Gpu, wgpu::SurfaceConfiguration)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let surface = instance.create_surface(window.clone())?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .context("no suitable GPU adapter")?;
    log::info!("adapter: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))?;
    let gpu = Gpu { device, queue };
    let caps = surface.get_capabilities(&adapter);
    let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
    let video = crate::video::VideoSettings::load();
    video.apply_window(window);
    let size = window.inner_size();
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: video.present_mode(),
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&gpu.device, &config);
    Ok((surface, gpu, config))
}

/// The window while the list of characters and the creation of one are up: the screens, and what they are drawn with. The game is
/// opened in the same window when one is chosen (and this is let go first, for the window can have only one surface).
struct GlueWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    renderer: Renderer,
    glue: Glue,
    /// The music and the sounds of the screens (none with `--no-audio`, or without a sound device).
    audio: Option<Audio>,
    mouse: (f32, f32),
    last: Instant,
}

impl GlueWindow {
    fn new(window: Arc<Window>, mount: Arc<Mount>, screen: Option<&str>, sound: bool) -> Result<GlueWindow> {
        let (surface, gpu, config) = open_gpu(&window)?;
        let mut renderer = Renderer::new(&gpu, config.format, Atmosphere::daytime(100.0));
        let audio = if sound { Audio::new(mount.clone()).map_err(|e| log::warn!("no audio: {e:#}")).ok() } else { None };
        let audio = audio.map(|mut a| {
            a.apply(&crate::audio::SoundSettings::load());
            a
        });
        let glue = Glue::new(mount, &mut renderer, &gpu, save::characters_dir(), screen)?;
        window.set_cursor_visible(!glue.has_cursor());
        Ok(GlueWindow { window, surface, config, gpu, renderer, glue, audio, mouse: (-1.0, -1.0), last: Instant::now() })
    }

    fn event(&mut self, event: &WindowEvent) -> Option<Outcome> {
        use winit::keyboard::{Key as Logical, NamedKey};
        match event {
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                (self.config.width, self.config.height) = (size.width, size.height);
                self.surface.configure(&self.gpu.device, &self.config);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
                self.glue.mouse_moved(self.mouse);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => self.glue.mouse_down(self.mouse),
                ElementState::Released => self.glue.mouse_up(self.mouse),
            },
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let key = match event.logical_key {
                    Logical::Named(NamedKey::Enter) => Some(GlueKey::Enter),
                    Logical::Named(NamedKey::Escape) => Some(GlueKey::Escape),
                    Logical::Named(NamedKey::Backspace) => Some(GlueKey::Backspace),
                    _ => None,
                };
                match (key, &event.text) {
                    (Some(key), _) => self.glue.key(key),
                    (None, Some(text)) => self.glue.typed(text),
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
        if let Some(audio) = &mut self.audio {
            for name in self.glue.take_sounds() {
                audio.play(Sfx::Named(name), 0);
            }
        }
        self.glue.outcome()
    }

    fn frame(&mut self) {
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        let size = (self.config.width, self.config.height);
        let Ok(frame) = self.surface.get_current_texture() else { return };
        let view = frame.texture.create_view(&Default::default());
        if let Some(audio) = &mut self.audio {
            audio.update_menu(dt);
        }
        self.glue.draw(&mut self.renderer, &self.gpu, size, dt);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("glue"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        }));
        self.renderer.ui.draw(&self.gpu, &mut encoder, &view, size);
        self.gpu.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

/// The window while the introduction of a race plays: the world seen from the fly-by camera, with the narration. Like the character list it
/// is let go before the game takes the window.
struct CinemaWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    renderer: Renderer,
    sky: Option<Sky>,
    world: World,
    weather: crate::weather::Weather,
    /// The hour of the game's day (it begins as a new game does and runs a minute a second), and what the weather looks up four times a second.
    hour: f32,
    indoors: bool,
    climate: Option<crate::weather::Kind>,
    weather_look: f32,
    last_eye: Vec3,
    flyby: crate::cinematic::Flyby,
    audio: Option<Audio>,
    far: f32,
    /// Milliseconds into the fly-by; it begins when the world around its start has been loaded.
    clock: f32,
    ready: bool,
    last: Instant,
}

impl CinemaWindow {
    fn new(window: Arc<Window>, mount: Arc<Mount>, race: crate::character::Race, args: &Args) -> Result<CinemaWindow> {
        let map = crate::cinematic::map_of(race);
        let flyby = crate::cinematic::Flyby::load(&mount, race)?;
        let (surface, gpu, config) = open_gpu(&window)?;
        let view_distance = args.radius as f32 * wowfmt::adt::TILE_SIZE * 0.95;
        let mut renderer = Renderer::new(&gpu, config.format, Atmosphere::daytime(view_distance));
        let sky = Sky::load(&mount, map, view_distance).map_err(|e| log::warn!("no light data: {e:#}")).ok();
        match crate::skygpu::SkyAssets::load(&mount) {
            Ok(assets) => renderer.set_sky(&gpu, &assets),
            Err(e) => log::warn!("no sky pictures: {e:#}"),
        }
        let audio = if args.audio { Audio::new(mount.clone()).map_err(|e| log::warn!("no audio: {e:#}")).ok() } else { None };
        let world = World::open(mount, map, args.radius)?;
        renderer.set_water_textures(&gpu, &world.water_textures());
        window.set_cursor_visible(false);
        log::info!("introduction of the {}: {} ({:.0} s)", race.name(), flyby.model, flyby.duration / 1000.0);
        Ok(CinemaWindow { window, surface, config, gpu, renderer, sky, world, weather: weather_at_start(args), hour: args.time.unwrap_or(12.0), indoors: false, climate: None, weather_look: 0.0, last_eye: flyby.camera_at(0.0, 1.0).pos, flyby, audio, far: view_distance * 1.2, clock: 0.0, ready: false, last: Instant::now() })
    }

    /// Whether it is over: the fly-by ended, or a key or the mouse said to skip it.
    fn event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                (self.config.width, self.config.height) = (size.width, size.height);
                self.surface.configure(&self.gpu.device, &self.config);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                return matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape | KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter));
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, .. } => return true,
            WindowEvent::RedrawRequested => return self.frame(),
            _ => {}
        }
        false
    }

    fn frame(&mut self) -> bool {
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        let size = (self.config.width, self.config.height);
        let mut camera = self.flyby.camera_at(self.clock, size.0 as f32 / size.1 as f32);
        camera.far = self.far;
        self.world.update(camera.pos.x, camera.pos.z, &self.gpu, &mut self.renderer);
        // The fly-by waits (black) for the world around its beginning, and then the narrator speaks.
        if !self.ready && self.world.pending_count() == 0 {
            self.ready = true;
            if let Some(audio) = &mut self.audio {
                audio.play(Sfx::Entry(self.flyby.sound), 0);
            }
        }
        if self.ready {
            self.clock += dt * 1000.0;
            self.hour = (self.hour + dt / 60.0) % 24.0;
        }
        // The weather of the zone, as in the game.
        let eye = camera.pos;
        self.weather_look -= dt;
        if self.weather_look <= 0.0 {
            self.weather_look = 0.25;
            self.indoors = self.world.indoors(eye);
            self.climate = self.world.area_at(eye.x, eye.z).and_then(|a| crate::weather::climate_of(&self.world.areas.display(a)));
        }
        let (sy, cy) = camera.yaw.sin_cos();
        let velocity = ((eye - self.last_eye) / dt.max(1e-3)).clamp_length_max(25.0);
        self.weather.update(dt, &crate::weather::Motion { eye, velocity, forward: Vec3::new(sy, 0.0, -cy), indoors: self.indoors }, &self.world, self.climate);
        self.last_eye = eye;
        if let Some(a) = self.sky.as_ref().and_then(|s| s.atmosphere(camera.pos, self.hour, self.weather.storm())) {
            self.renderer.atmosphere = a;
        }
        if let Some(audio) = &mut self.audio {
            audio.update(dt, self.world.area_at(camera.pos.x, camera.pos.z), self.hour);
            audio.set_weather(self.weather.sound(), self.indoors, dt);
        }
        let Ok(frame) = self.surface.get_current_texture() else { return false };
        let view = frame.texture.create_view(&Default::default());
        if self.ready {
            self.renderer.prepare_weather(&self.gpu, &self.weather, &camera, size);
            self.renderer.render(&self.gpu, &view, size, &camera, self.weather.clock, self.world.tiles());
        } else {
            let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cinema wait"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            }));
            self.gpu.queue.submit(Some(encoder.finish()));
        }
        frame.present();
        self.clock >= self.flyby.duration
    }
}

/// Show one frame of the loading screen.
fn present_loading(surface: &wgpu::Surface, gpu: &Gpu, renderer: &mut Renderer, loading: &Loading, size: (u32, u32)) {
    let Ok(frame) = surface.get_current_texture() else { return };
    let view = frame.texture.create_view(&Default::default());
    loading.draw(&mut renderer.ui, size);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    // The picture covers everything; the pass only makes the target ready.
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("loading"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    }));
    renderer.ui.draw(gpu, &mut encoder, &view, size);
    gpu.queue.submit(Some(encoder.finish()));
    frame.present();
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

/// The creatures of `content/spawns.ron`: (kind, feet position in render space, heading in radians).
/// The weather at the start: fixed by `--weather`, none with `--no-weather`, otherwise it comes and goes by the zone's climate.
fn weather_at_start(args: &Args) -> crate::weather::Weather {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let mut weather = crate::weather::Weather::new(seed);
    match args.weather {
        Some((kind, strength)) => {
            weather.auto = false;
            weather.set(kind, strength, true);
        }
        None => weather.auto = !args.no_weather,
    }
    weather
}

/// The places of the props that are beds in `map`.
fn bed_spots(props: &[content::Prop], map: &str) -> Vec<Vec3> {
    props.iter().filter(|p| p.sleep && p.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map))).map(|p| crate::coords::wow_to_render([p.at.0, p.at.1, p.at.2])).collect()
}

/// Whether a bed is within reach of `feet`.
fn near_bed(beds: &[Vec3], world: &World, feet: Vec3) -> bool {
    world.bed_near(feet, BED_RANGE, BED_HEIGHT) || beds.iter().any(|b| (b.x - feet.x).hypot(b.z - feet.z) <= BED_RANGE && (b.y - feet.y).abs() < BED_HEIGHT)
}

/// The props of the content that are picked up.
fn pickup_props() -> Vec<content::Prop> {
    content::get().props.iter().filter(|p| p.pickup.is_some()).cloned().collect()
}

fn spawn_list() -> Vec<(usize, Vec3, f32)> {
    content::get().spawns.iter().map(|s| (content::creature_index(&s.creature), wow_to_render([s.at.0, s.at.1, 0.0]), s.facing.to_radians())).collect()
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
    wolves: Option<Creatures>,
    chests: Option<Chests>,
    /// The props that quests send the player to pick up, and the map they are in.
    pickups: Option<Pickups>,
    /// The props that are picked up (the world's props change in the editor), and whether `pickups` is to be made again from them.
    pickup_props: Vec<content::Prop>,
    pickups_stale: bool,
    /// Where the beds are (render space): the props flagged `sleep`.
    beds: Vec<Vec3>,
    map: String,
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
    restart: bool,
    /// The restart goes back to the list of characters, not into the game.
    logout: bool,
    /// The game menu's Exit Game was clicked.
    quit: bool,
    /// The present mode changed: the surface is configured again after the frame.
    reconfigure: bool,
    /// The names over creatures show (the interface options).
    names: bool,
    /// The editor asked for the content it saved to be taken into use (and when to ask it by itself: `--apply-after`).
    apply: bool,
    apply_after: Option<f32>,
    mount: Arc<Mount>,
    /// The level up effect while it plays, and whether the character moved this frame (a cast fails when he does).
    levelfx: Option<usize>,
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
    climate: Option<crate::weather::Kind>,
    weather_look: f32,
    /// What is heard from places (streams, waterfalls, fountains), looked for four times a second, and whether the feet are in water.
    places: Vec<(u64, u32, Vec3)>,
    wading: bool,
    /// The ripples the character makes in water, what draws them, how deep the water was last frame, how fast the character goes, and whether it took a
    /// step since the spray was last looked for.
    ripples: crate::ripples::Ripples,
    ripple_draw: Option<crate::ripples::RippleDraw>,
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
}

impl State {
    fn new(window: Arc<Window>, args: &Args, mount: Arc<Mount>) -> Result<State> {
        let (surface, gpu, config) = open_gpu(&window)?;
        let format = config.format;

        let view_distance = args.radius as f32 * wowfmt::adt::TILE_SIZE * 0.95;
        let mut renderer = Renderer::new(&gpu, format, Atmosphere::daytime(view_distance));
        // The loading screen is up while the rest is made ready (and after, until the terrain around the spawn has streamed in).
        let mut loading = Loading::load(&mount, &args.map, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no loading screen: {e:#}")).ok();
        let mut show = |renderer: &mut Renderer, reached: &[(Stage, f32)]| {
            if let Some(loading) = &mut loading {
                reached.iter().for_each(|&(stage, value)| loading.report(stage, value));
                present_loading(&surface, &gpu, renderer, loading, (config.width, config.height));
            }
        };
        show(&mut renderer, &[]);
        let sky = Sky::load(&mount, &args.map, view_distance).map_err(|e| log::warn!("no light data: {e:#}")).ok();
        match crate::skygpu::SkyAssets::load(&mount) {
            Ok(assets) => renderer.set_sky(&gpu, &assets),
            Err(e) => log::warn!("no sky pictures: {e:#}"),
        }
        match crate::weathergpu::WeatherAssets::load(&mount) {
            Ok(assets) => renderer.set_weather_assets(&gpu, &assets),
            Err(e) => log::warn!("no weather pictures: {e:#}"),
        }
        let player_mount = mount.clone();
        let audio = if args.audio { Audio::new(mount.clone()).map_err(|e| log::warn!("no audio: {e:#}")).ok() } else { None };
        let audio = audio.map(|mut a| {
            a.apply(&crate::audio::SoundSettings::load());
            a
        });
        let world = World::open(mount, &args.map, args.radius)?;
        renderer.set_water_textures(&gpu, &world.water_textures());
        show(&mut renderer, &[]);
        // The saved game, if there is one: a broken file is left alone, and then nothing is saved this time.
        let mut save_path = args.save.then(|| args.save_path.clone().unwrap_or_else(|| save::character_path(args.character.as_deref().unwrap_or(DEFAULT_NAME))));
        let loaded = match save_path.as_ref().filter(|_| !args.new_game).map(|p| save::read(p)) {
            Some(Ok(saved)) => saved,
            Some(Err(e)) => {
                log::warn!("{e:#}; starting a new game and not saving it");
                save_path = None;
                None
            }
            None => None,
        };
        let look = loaded.as_ref().map_or(args.look, |s| s.look);
        let name = args.character.clone().unwrap_or_else(|| DEFAULT_NAME.to_string());
        let mut player = Player::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &look).map_err(|e| log::warn!("no player: {e:#}")).ok();

        let mut actionbar = ActionBar::load(&player_mount, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no action bar: {e:#}")).ok();

        show(&mut renderer, &[(Stage::Objects, 1.0 / 3.0), (Stage::Interface, 0.2)]);
        let mut panels = Panels::new(player_mount.clone(), &mut renderer, &gpu).map_err(|e| log::warn!("no character sheet: {e:#}")).ok();
        if let Some(p) = &mut panels {
            p.set_video_modes(crate::video::VideoSettings::modes(&window));
        }
        show(&mut renderer, &[(Stage::Interface, 0.5)]);
        if let (Some(saved), Some(pl)) = (&loaded, &mut panels) {
            log::info!("loaded the saved game: level {}", saved.level());
            pl.restore(saved);
        }
        if let Some(pl) = &mut panels {
            pl.set_identity(&name, look.race);
        }
        if let (Some(bar), Some(pl)) = (&mut actionbar, &panels) {
            match loaded.as_ref().filter(|s| !s.bar.is_empty()) {
                Some(saved) => bar.restore(&saved.bar),
                None => pl.known.iter().for_each(|&spell| bar.learn(spell)),
            }
        }
        if let (Some(p), Some(pl)) = (&mut player, &panels) {
            p.set_equipment(&mut renderer, &gpu, &pl.inv.equipment, &pl.stats);
            if let Some(saved) = &loaded {
                // Alive, with the share of health the character had.
                p.vitals.health.current = (saved.health.clamp(0.0, 1.0) * p.vitals.health.max).max(1.0);
                p.vitals.mana.current = saved.mana.clamp(0.0, 1.0) * p.vitals.mana.max;
                p.vitals.stamina.current = saved.stamina.clamp(0.0, 1.0) * p.vitals.stamina.max;
                p.set_sheathed(saved.sheathed);
                // Back in the saddle (a steed that no longer loads leaves the character on foot).
                if let Some((display, percent)) = saved.steed {
                    if let Err(e) = p.ride(display, percent, &mut renderer, &gpu) {
                        log::warn!("the saved steed does not load: {e:#}");
                    }
                }
                p.set_sneaking(saved.sneaking);
            }
        }
        let mut chests = Chests::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu).map_err(|e| log::warn!("no chests: {e:#}")).ok();
        if let (Some(chests), Some(saved)) = (&mut chests, &loaded) {
            chests.restore(&saved.chests);
        }
        let pickup_props = pickup_props();
        let pickups = Pickups::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &pickup_props, &args.map).map_err(|e| log::warn!("no pickups: {e:#}")).ok();
        let lock_art = crate::lockpick::LockArt::load(&player_mount, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no lockpicking pictures: {e:#}")).ok();
        // The pointer of the original replaces the system's, which is shown only if its pictures are missing.
        let cursors = Cursors::load(&player_mount, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no mouse pointer: {e:#}")).ok();
        window.set_cursor_visible(cursors.is_none());
        show(&mut renderer, &[(Stage::Objects, 2.0 / 3.0), (Stage::Interface, 0.7)]);
        let effects = crate::effects::Effects::new(player_mount.clone());
        let ripple_draw = crate::ripples::RippleDraw::new(&player_mount, &ModelCache::default(), &mut renderer, &gpu);
        let editor = player_mount
            .read(r"Fonts\ARIALN.TTF")
            .map_err(anyhow::Error::from)
            .and_then(|bytes| fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|e| anyhow::anyhow!("{e}")))
            .and_then(|font| crate::editor::Editor::new(&mut renderer, &gpu, &font, player_mount.clone()))
            .map_err(|e| log::warn!("no editor: {e:#}"))
            .ok();
        let mut editor = editor;
        if let (Some(editor), Some((tab, n, _))) = (&mut editor, &args.editor) {
            editor.show(tab, *n);
        }
        let console = fontdue::Font::from_bytes(player_mount.read(r"Fonts\ARIALN.TTF")?, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("{e}"))
            .and_then(|font| crate::console::Console::new(&mut renderer.ui, &gpu, &font))
            .map_err(|e| log::warn!("no console: {e:#}"))
            .ok();
        let caster = crate::casting::Spellcaster::new(crate::visuals::load(&player_mount).map_err(|e| log::warn!("no spell visuals: {e:#}")).unwrap_or_default());
        let foes = crate::foes::Foes::new(crate::visuals::load_classic(&player_mount).map_err(|e| log::warn!("no creature spell visuals: {e:#}")).unwrap_or_default());
        let combat_text = CombatText::load(&player_mount, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no combat text: {e:#}")).ok();

        let (x, y, z) = args.pos;
        let spawn = wow_to_render([x, y, z.unwrap_or(0.0)]);
        // Where the saved character stood (it drops onto the ground there), or the start.
        let (start, yaw, pitch) = match &loaded {
            Some(saved) => (wow_to_render([saved.position.0, saved.position.1, 0.0]), saved.heading.to_radians(), saved.pitch.to_radians()),
            None => (spawn, args.yaw_deg.to_radians(), args.pitch_deg.to_radians()),
        };
        let mut camera = Camera::new(start, yaw, pitch);
        // The wolves of `content/spawns.ron`.
        let mut wolves = Creatures::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &spawn_list()).map_err(|e| log::warn!("no creatures: {e:#}")).ok();
        if let Some(wolves) = &mut wolves {
            wolves.set_spell_anims(foes.anims());
        }
        if let (Some(wolves), Some(saved)) = (&mut wolves, &loaded) {
            wolves.restore(&saved.mobs);
        }
        camera.aspect = config.width as f32 / config.height as f32;
        camera.far = view_distance * 1.2;
        show(&mut renderer, &[(Stage::Objects, 1.0), (Stage::Interface, 1.0)]);
        let mode = if args.fly { Mode::Fly } else { Mode::Walk };

        Ok(State {
            name,
            look,
            loading,
            spare_loading: None,
            window,
            surface,
            config,
            gpu,
            renderer,
            sky,
            player,
            audio,
            actionbar,
            wolves,
            chests,
            pickups,
            pickup_props,
            pickups_stale: false,
            beds: bed_spots(&content::get().props, &args.map),
            map: args.map.clone(),
            lock: None,
            lock_turn: false,
            lock_time: 0.0,
            lock_art,
            effects,
            editor,
            console,
            god: false,
            detached: false,
            restart: false,
            logout: false,
            quit: false,
            reconfigure: false,
            names: true,
            apply: false,
            apply_after: args.apply_after,
            mount: player_mount.clone(),
            levelfx: None,
            moving: false,
            caster,
            foes,
            fires: Default::default(),
            cast_request: None,
            hint: None,
            cursors,
            hover: None,
            pointer: Kind::Point,
            combat_text,
            panels,
            in_combat: false,
            quiet: 0.0,
            low_health: false,
            spawn: (spawn.x, spawn.z),
            mouse_slot: None,
            cursor: None,
            last_eye: start,
            hour: args.time.or(loaded.as_ref().and_then(|s| s.hour)).unwrap_or(12.0),
            fixed_time: args.time.is_some(),
            weather: weather_at_start(&args),
            indoors: false,
            climate: None,
            weather_look: 0.0,
            places: Vec::new(),
            wading: false,
            ripples: Default::default(),
            ripple_draw,
            water_depth: 0.0,
            speed: 0.0,
            stepped: false,
            sound_look: 0.0,
            world,
            camera,
            eye: None,
            free: None,
            looking: false,
            rotating: false,
            ctrl: false,
            controller: Controller::new(mode),
            input: Input::default(),
            snap_to_ground: (z.is_none() || loaded.is_some()).then_some(0.0),
            saved_feet: loaded.as_ref().and_then(|s| s.height),
            save_path,
            last_save: Instant::now(),
            indoor_check: 0.0,
            grabbed: false,
            start: Instant::now(),
            last_frame: Instant::now(),
            title_timer: Instant::now(),
            frames: 0,
            render_time: Duration::ZERO,
        })
    }

    /// The character as it is now, by ids: where it stands, what it carries, its level, quests, skills and discoveries.
    fn capture(&self) -> Option<save::Save> {
        let (panels, player) = (self.panels.as_ref()?, self.player.as_ref()?);
        let [north, west, _] = render_to_wow(self.camera.pos);
        let health = player.vitals.health.fraction().max(0.05);
        let mut saved = save::Save::capture(&panels.inv, panels.progress, &panels.quests, (north, west), self.camera.yaw.to_degrees(), self.camera.pitch.to_degrees(), health);
        saved.skills = panels.skills.parts();
        saved.spells = panels.known.iter().map(|&i| content::spell(i).id.clone()).collect();
        saved.bar = self.actionbar.as_ref().map_or_else(Vec::new, |b| b.layout());
        saved.discovered = content::get().pois.iter().zip(&panels.found).filter(|(_, &f)| f).map(|(p, _)| p.id.clone()).collect();
        saved.explored = panels.explored.clone();
        saved.hour = (!self.fixed_time).then_some(self.hour);
        (saved.mana, saved.stamina) = (player.vitals.mana.fraction(), player.vitals.stamina.fraction());
        saved.cooldowns = content::get().spells.iter().zip(&panels.cooldowns).filter(|(_, &left)| left > 0.0).map(|(s, &left)| (s.id.clone(), left)).collect();
        saved.mobs = self.wolves.as_ref().map_or_else(Vec::new, |w| w.snapshot());
        saved.chests = self.chests.as_ref().map_or_else(Vec::new, |c| c.snapshot());
        saved.steed = player.steed_info();
        saved.sheathed = player.is_sheathed();
        saved.height = Some(self.camera.pos.y - EYE_HEIGHT);
        (saved.buffs, saved.item_cooldowns, saved.hearth) = panels.consumed.snapshot();
        saved.sneaking = player.is_sneaking();
        saved.look = self.look;
        Some(saved)
    }

    /// Write the saved game: the character, what it carries, its level and quests.
    fn save_game(&mut self) {
        self.last_save = Instant::now();
        let (Some(path), Some(saved)) = (&self.save_path, self.capture()) else { return };
        if let Err(e) = save::write(path, &saved) {
            log::warn!("cannot save the game: {e:#}");
        }
    }

    /// Take the content files (as the editor saved them) into use while the game runs: the character is kept, by ids, and the creatures and
    /// chests are put in the world again from the new content (corpses, loot and wounds are forgotten, like after a respawn). If that
    /// cannot be done (the spells changed), the game starts again instead.
    fn apply_content(&mut self) {
        let started = Instant::now();
        let live = self.reload_content();
        match &live {
            Ok(()) => log::info!("applied the content live in {:?}", started.elapsed()),
            Err(e) => {
                log::warn!("cannot apply the content live: {e:#}");
                self.restart = true;
            }
        }
        if let Some(editor) = &mut self.editor {
            editor.applied(&mut self.renderer, live.is_ok());
        }
    }

    fn reload_content(&mut self) -> Result<()> {
        let saved = self.capture();
        content::reload(&self.mount)?;
        self.world.set_props(content::get().props.clone());
        if let Some(old) = self.wolves.take() {
            old.retire(&mut self.renderer);
        }
        if let Some(old) = self.chests.take() {
            old.retire(&mut self.renderer);
        }
        if let Some(old) = self.pickups.take() {
            old.retire(&mut self.renderer);
        }
        (self.lock, self.hover, self.hint, self.cast_request) = (None, None, None, None);
        if let Some(panels) = &mut self.panels {
            panels.forget_world();
            if let Some(saved) = &saved {
                panels.restore(saved);
            }
        }
        if let (Some(player), Some(panels)) = (&mut self.player, &self.panels) {
            player.forget_items();
            player.set_equipment(&mut self.renderer, &self.gpu, &panels.inv.equipment, &panels.stats);
        }
        if let Some(audio) = &mut self.audio {
            audio.bank.reload_cries(&self.mount).unwrap_or_else(|e| log::warn!("no cries: {e:#}"));
        }
        self.wolves = Creatures::load(&self.mount, &ModelCache::default(), &mut self.renderer, &self.gpu, &spawn_list()).map_err(|e| log::warn!("no creatures: {e:#}")).ok();
        if let Some(wolves) = &mut self.wolves {
            wolves.set_spell_anims(self.foes.anims());
        }
        self.chests = Chests::load(&self.mount, &ModelCache::default(), &mut self.renderer, &self.gpu).map_err(|e| log::warn!("no chests: {e:#}")).ok();
        self.pickup_props = pickup_props();
        self.beds = bed_spots(&content::get().props, &self.map);
        self.load_pickups();
        Ok(())
    }

    /// Put the props to pick up in the world, in place of those there were (the props changed).
    fn load_pickups(&mut self) {
        if let Some(old) = self.pickups.take() {
            old.retire(&mut self.renderer);
        }
        self.pickups = Pickups::load(&self.mount, &ModelCache::default(), &mut self.renderer, &self.gpu, &self.pickup_props, &self.map).map_err(|e| log::warn!("no pickups: {e:#}")).ok();
    }

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

    /// Moves the weather along; where the player is and what the zone allows are looked up four times a second.
    fn update_weather(&mut self, dt: f32) {
        let eye = self.camera.pos;
        self.weather_look -= dt;
        if self.weather_look <= 0.0 {
            self.weather_look = 0.25;
            self.indoors = self.world.indoors(eye);
            self.climate = self.world.area_at(eye.x, eye.z).and_then(|a| crate::weather::climate_of(&self.world.areas.display(a)));
        }
        let (sy, cy) = self.camera.yaw.sin_cos();
        // A jump of the camera (the first frames, a teleport) is not a speed.
        let velocity = ((eye - self.last_eye) / dt.max(1e-3)).clamp_length_max(25.0);
        let motion = crate::weather::Motion { eye, velocity, forward: Vec3::new(sy, 0.0, -cy), indoors: self.indoors };
        self.weather.update(dt, &motion, &self.world, self.climate);
    }

    /// Looks for what is heard from places round the ears: the liquid within nine yards (the nearest of each kind, at most two kinds, as the original
    /// does) and the models that play a sound all the time (the six nearest within their reach); and whether the feet are in water.
    fn look_for_sounds(&mut self) {
        let Some(bank) = self.audio.as_ref().map(|a| &a.bank) else { return };
        let ears = self.camera.pos;
        let mut places = Vec::new();
        for (kind, speeds) in self.world.nearby_liquids(ears, 9.0).iter().enumerate() {
            let nearest = speeds.iter().enumerate().find_map(|(speed, at)| at.map(|at| (speed, at)));
            if let Some((entry, at)) = nearest.and_then(|(speed, at)| bank.water_sound(kind, speed).map(|e| (e, at))) {
                if places.len() < 2 {
                    places.push(((1u64 << 60) | kind as u64, entry, at));
                }
            }
        }
        let mut sources = self.world.nearby_sounds(ears, 130.0);
        sources.sort_by(|a, b| a.0.distance(ears).total_cmp(&b.0.distance(ears)));
        places.extend(sources.into_iter().take(6).map(|(at, entry)| {
            let key = ((at.x * 8.0) as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((at.z * 8.0) as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ entry as u64;
            (2u64 << 60 | key >> 4, entry, at)
        }));
        self.places = places;
        let feet = ears - Vec3::Y * EYE_HEIGHT;
        self.wading = self.world.liquid_at(feet.x, feet.z, feet.y).is_some_and(|level| level > feet.y + 0.1);
    }

    /// What the character does to the water it is in: the spray of a footstep in shallow water (`FUN_005fbf70`: less than half its height deep, walking
    /// or running by the speed, at the surface), and the ripples (`FUN_005fa760`, started also by `FUN_006030c0` when the water crosses 0.4 of the
    /// height) that `ripples.rs` makes and draws.
    fn water_effects(&mut self, dt: f32, camera: Vec3) {
        use crate::ripples::{State, Walker};
        const SPRAY_RUN: &str = r"Particles\FootStepSprayWater.m2";
        const SPRAY_WALK: &str = r"Particles\FootStepSprayWaterWalk.m2";
        let size = self.player.as_ref().map_or_else(Default::default, |p| p.size());
        let height = size.height;
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        // The camera under the surface (its lowest edge of the near plane, 0.2 yd under it) hears the loop of the sea.
        let under = self.world.liquid_at(camera.x, camera.z, camera.y).is_some_and(|level| camera.y - 0.2 < level);
        if let Some(audio) = &mut self.audio {
            audio.set_underwater(under);
        }
        let level = self.input.water.filter(|_| self.controller.mode == Mode::Walk);
        let depth = level.map_or(0.0, |level| level - feet.y);
        let swimming = self.controller.swimming();
        let stepped = std::mem::take(&mut self.stepped);
        if let (Some(level), true) = (level, stepped && !swimming && depth > 0.0 && depth < 0.5 * height) {
            // (the original: running when faster than twice the walking speed, 2.5)
            let run = self.speed > 5.0;
            let at = glam::Mat4::from_translation(Vec3::new(feet.x, level, feet.z));
            self.effects.spawn(&mut self.renderer, &self.gpu, if run { SPRAY_RUN } else { SPRAY_WALK }, false, at);
        }
        self.ripples.update(dt);
        if let Some(level) = level {
            // Where it goes: the way it faces, turned by the keys (the original's offsets: a strafe is a quarter turn, with forward or back an eighth
            // or three eighths, and backing is half a turn).
            let (i, quarter) = (&self.input, std::f32::consts::FRAC_PI_4);
            let turn = if i.left {
                if i.forward { quarter } else if i.back { 3.0 * quarter } else { 2.0 * quarter }
            } else if i.right {
                -if i.forward { quarter } else if i.back { 3.0 * quarter } else { 2.0 * quarter }
            } else if i.back && !i.forward {
                4.0 * quarter
            } else {
                0.0
            };
            let yaw = self.camera.yaw - turn;
            let moving = i.forward || i.back || i.left || i.right;
            // The water reaching 0.4 of the height, either way, makes a splash (not for a swimmer).
            let crossed = (depth > 0.4 * height) != (self.water_depth > 0.4 * height) && !swimming;
            let state = if crossed { State::Entering } else if moving { State::Moving } else { State::Idle };
            if crossed {
                if let Some(audio) = &mut self.audio {
                    audio.splash(height);
                }
            }
            let walker = Walker { feet, level, height, scale: size.scale, speed: self.speed, heading: (yaw.sin(), -yaw.cos()) };
            let world = &self.world;
            self.ripples.make(&walker, state, |at, half| world.liquid_quads(at.x, at.z, feet.y, half));
        }
        self.water_depth = depth;
        if let Some(draw) = &mut self.ripple_draw {
            draw.draw(&self.ripples, feet, &mut self.renderer, &self.gpu);
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        // The game saves itself now and then (and when it ends).
        if self.last_save.elapsed() >= Duration::from_secs(SAVE_SECONDS) {
            self.save_game();
        }

        // While the editor is open its camera flies free: it starts behind and above the player, looking where he looks.
        if !self.detached && !self.editor.as_ref().is_some_and(|e| e.open) {
            self.free = None;
        } else if self.free.is_none() {
            let (forward, mut controller) = (self.camera.forward(), Controller::new(Mode::Fly));
            controller.fly_speed = FREE_SPEED;
            self.free = Some(FreeCam { camera: Camera { pos: self.camera.pos - forward * 5.0 + Vec3::Y * 1.5, ..self.camera }, controller });
        }
        let watching = self.free.as_ref().map_or(self.camera.pos, |f| f.camera.pos);
        self.world.update(watching.x, watching.z, &self.gpu, &mut self.renderer);
        if !self.fixed_time {
            // One game minute per second, like the original.
            self.hour = (self.hour + dt / 60.0) % 24.0;
        }
        self.update_weather(dt);
        if let Some(a) = self.sky.as_ref().and_then(|s| s.atmosphere(self.camera.pos, self.hour, self.weather.storm())) {
            self.renderer.atmosphere = a;
        }

        // First-frame spawn: drop onto the terrain as soon as it is available.
        if let Some(_) = self.snap_to_ground {
            // With a saved height, wait for the surrounding tiles too: their buildings hold the floor.
            let (x, z) = (self.camera.pos.x, self.camera.pos.z);
            let ground = match self.saved_feet {
                Some(_) if self.world.pending_count() > 0 => None,
                Some(feet) => crate::collision::Collider::floor(&self.world, x, z, feet + 1.0).or_else(|| self.world.height_at(x, z)),
                None => self.world.height_at(x, z),
            };
            if let Some(ground) = ground {
                let lift = if self.controller.mode == Mode::Fly { 30.0 } else { EYE_HEIGHT };
                self.camera.pos.y = ground + lift;
                log::info!("spawned at height {ground:.2} (saved {:?}, terrain {:?})", self.saved_feet, self.world.height_at(x, z));
                self.snap_to_ground = None;
                self.saved_feet = None;
            }
        } else {
            // Shift sprints (on foot or in the saddle) for as long as there is stamina.
            let sneaking = self.controller.mode == Mode::Walk && self.player.as_ref().is_some_and(|p| p.is_sneaking());
            self.input.sneak = sneaking;
            // The water under the feet (deep enough, the character swims: stamina, tiredness and the skill in swimming are the player's).
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            self.input.water = self.world.liquid_at(feet.x, feet.z, feet.y);
            if let Some(p) = &self.player {
                self.controller.set_height(p.size().height);
            }
            let swimming = self.controller.swimming();
            self.input.tired = self.player.as_ref().is_some_and(|p| p.is_worn_out());
            self.input.sprint = self.input.fast && self.controller.mode == Mode::Walk && !sneaking && !swimming && self.player.as_ref().is_none_or(|p| p.can_sprint());
            self.input.mount_bonus = self.player.as_ref().map_or(0.0, |p| p.speed_bonus());
            self.input.burden = self.panels.as_ref().map_or(0.0, |pn| pn.burden());
            // Spells that have hold of the character: slowed, rooted, stunned, or running in fear from where the spell came from.
            let held = &self.foes.afflictions;
            self.input.burden = self.input.burden.max(held.slow());
            self.caster.set_blocked(held.cast_blocked());
            if let Some(p) = &mut self.player {
                p.set_held_back(held.swing_blocked(), held.shot_blocked());
            }
            if held.stunned() || held.rooted() || held.feared().is_some() {
                (self.input.forward, self.input.back, self.input.left, self.input.right, self.input.jump_pressed) = (false, false, false, false, false);
            }
            if let Some(from) = held.feared() {
                let want = (feet.x - from.x).atan2(-(feet.z - from.z));
                let turn = (want - self.camera.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                self.camera.yaw = (self.camera.yaw + turn.clamp(-3.0 * dt, 3.0 * dt)).rem_euclid(std::f32::consts::TAU);
                self.input.forward = true;
            }
            if let Some(p) = &mut self.player {
                p.set_sprinting(self.input.sprint);
                p.set_swimming(swimming, self.panels.as_ref().map_or(1.0, |pn| pn.skills.swim_cost_factor()));
                // A jump on foot costs stamina; without enough of it the character stays on the ground.
                if self.input.jump_pressed && self.controller.mode == Mode::Walk && !self.controller.airborne() && !swimming && !p.try_jump() {
                    self.input.jump_pressed = false;
                } else if self.input.jump_pressed {
                    // Jumping stands up.
                    p.set_sneaking(false);
                }
            }
            // No walking, turning or jumping while a lock is being picked.
            if let Some(free) = &mut self.free {
                free.controller.update(&mut free.camera, &mut self.input, &self.world, dt);
            } else if self.lock.is_none() {
                self.controller.update(&mut self.camera, &mut self.input, &self.world, dt);
            }
        }

        if let Some(loading) = &mut self.loading {
            let pending = self.world.pending_count();
            loading.hold -= dt;
            if self.snap_to_ground.is_none() && pending == 0 && loading.hold <= 0.0 {
                self.spare_loading = self.loading.take();
            } else {
                // The terrain step of the bar: how many of the tiles that waited have been fetched.
                loading.peak = loading.peak.max(pending);
                loading.report(Stage::Terrain, 1.0 - pending as f32 / loading.peak.max(1) as f32);
                present_loading(&self.surface, &self.gpu, &mut self.renderer, loading, (self.config.width, self.config.height));
                return;
            }
        }
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.gpu.device, &self.config);
                return;
            }
            Err(e) => {
                log::warn!("surface error: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut render_camera = Camera { pos: self.camera.pos, ..self.camera };
        if let Some(audio) = &mut self.audio {
            audio.update(dt, self.world.area_at(self.camera.pos.x, self.camera.pos.z), self.hour);
            audio.set_weather(self.weather.sound(), self.indoors, dt);
        }
        self.sound_look -= dt;
        if self.sound_look <= 0.0 {
            self.sound_look = 0.1;
            self.look_for_sounds();
        }
        if let Some(audio) = &mut self.audio {
            audio.set_listener(self.camera.pos, self.camera.yaw);
            audio.set_places(&self.places);
            audio.set_wading(self.wading);
        }
        if let Some(player) = &mut self.player {
            let flat = (self.camera.pos - self.last_eye) * Vec3::new(1.0, 0.0, 1.0) / dt.max(1e-3);
            let (sy, cy) = self.camera.yaw.sin_cos();
            // Flying is not walking: no footsteps, the jump pose.
            let airborne = self.controller.airborne() || self.controller.mode == Mode::Fly;
            let swimming = self.controller.swimming();
            let motion = Motion { speed: flat.length(), forward_speed: flat.dot(Vec3::new(sy, 0.0, -cy)), side_speed: flat.dot(Vec3::new(cy, 0.0, sy)), airborne, swimming, ..Default::default() };
            let world = &self.world;
            render_camera = player.update(dt, &self.camera, motion, |from, to| world.camera_distance(from, to), &mut self.renderer, &self.gpu);
            self.moving = motion.speed > 0.5 || airborne;
            self.speed = motion.speed;
            // Swimming trains the skill in swimming; so does treading water, less.
            if let (true, Some(panels)) = (swimming, &mut self.panels) {
                panels.train(crate::skills::Skill::Swimming, dt / if motion.speed > 0.5 || (self.camera.pos.y - self.last_eye.y).abs() > 0.01 { 3.0 } else { 12.0 });
            }
            // Sneaking about (not standing still) trains the skill.
            if let (true, Some(panels)) = (player.is_sneaking() && motion.speed > 0.5 && !airborne, &mut self.panels) {
                panels.train(crate::skills::Skill::Sneaking, dt / 3.0);
            }
            let events = player.take_events();
            self.stepped |= events.contains(&Sfx::Footstep);
            if let Some(audio) = &mut self.audio {
                let surface = world.surface_under(self.camera.pos - Vec3::Y * EYE_HEIGHT) as u32;
                events.into_iter().for_each(|sfx| audio.play(sfx, surface));
            }
        }
        if let Some(free) = &self.free {
            render_camera = Camera { pos: free.camera.pos, ..free.camera };
        }
        self.last_eye = self.camera.pos;
        self.eye = Some(crate::propedit::Eye::of(&render_camera, (self.config.width, self.config.height)));
        self.water_effects(dt, render_camera.pos);
        let size = (self.config.width, self.config.height);
        if let Some(panels) = &mut self.panels {
            panels.tick_cooldowns(dt);
            self.cast_request = panels.take_spell_click().or(self.cast_request);
            if let Some(bar) = &mut self.actionbar {
                panels.take_learned().into_iter().for_each(|spell| bar.learn(spell));
                if let Some(item) = bar.take_item_use() {
                    panels.use_item_of(item);
                }
                // (An action lifted off the bar is carried by the bar itself.)
                if let Some(a) = panels.dragging() {
                    bar.set_drag(Some(a));
                }
            }
        }
        // The spells: one asked for, the one being cast, the missiles in flight, and the effects they are made of.
        if let (Some(player), Some(panels)) = (&mut self.player, &mut self.panels) {
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            let mut ctx = crate::casting::Ctx { player, panels, wolves: self.wolves.as_mut(), text: self.combat_text.as_mut(), effects: &mut self.effects, renderer: &mut self.renderer, gpu: &self.gpu };
            if let Some(spell) = self.cast_request.take() {
                self.caster.begin(&mut ctx, spell, feet);
            }
            if let Some(shot) = ctx.player.take_shot() {
                self.caster.shoot(&mut ctx, shot, feet, self.camera.yaw);
            }
            self.caster.update(&mut ctx, dt, feet, self.moving);
        }
        self.fires.update(&self.world, render_camera.pos, &mut self.effects, &mut self.renderer, &self.gpu);
        self.effects.update(dt, &render_camera, &mut self.renderer, &self.gpu);
        // Nothing can hurt the player while the editor is open.
        let safe = self.god || self.free.is_some();
        if let (Some(wolves), Some(player)) = (&mut self.wolves, &mut self.player) {
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            if let Some(why) = player.take_refusal() {
                if let Some(panels) = &mut self.panels {
                    panels.error(why);
                }
            }
            if let Some((swing, power)) = player.take_swing_hit() {
                let skill = self.panels.as_ref().map_or(crate::skills::Skill::Unarmed, |p| p.weapon_skill());
                let factor = self.panels.as_ref().map_or(1.0, |p| p.skills.damage_factor(skill));
                let heavy = if swing == crate::player::Swing::Heavy { crate::player::HEAVY_DAMAGE } else { 1.0 };
                // A swing that comes soon after the last one does only the share of its damage that the time between is of the weapon's speed.
                let damage = (player.roll_damage() * factor * heavy * power).round().max(1.0);
                // A sneaking player's first blow on a creature that has not noticed him is heavier.
                let hits = wolves.strike(feet, self.camera.yaw, damage, if player.is_sneaking() { SNEAK_ATTACK } else { 1.0 });
                if !hits.is_empty() {
                    player.emit(Sfx::SwordHit);
                    if let Some(panels) = &mut self.panels {
                        panels.train(skill, heavy * power);
                    }
                }
                for (at, dealt, sneak) in hits {
                    if let Some(text) = &mut self.combat_text {
                        text.damage_dealt(at, dealt as u32);
                    }
                    if let (true, Some(panels)) = (sneak, &mut self.panels) {
                        panels.notice("Sneak attack!");
                    }
                }
            }
            let world = &self.world;
            if let Some(panels) = &self.panels {
                wolves.set_markers(render_camera.pos, |kind| panels.marker(kind));
            }
            // The target frame follows the selected creature.
            let view = wolves.target_view(feet);
            if let Some(panels) = &mut self.panels {
                panels.set_target(view);
            }
            // A creature that starts at no height (a new game) finds no floor within a yard of it: it takes the terrain.
            let auras = self.foes.afflictions.spells();
            let watch = crate::creature::Watch {
                feet,
                yaw: self.camera.yaw,
                health: player.vitals.health.fraction(),
                mana: player.vitals.mana.fraction(),
                casting: self.caster.casting(),
                auras: &auras,
                sneaking: player.is_sneaking().then(|| self.panels.as_ref().map_or(1.0, |p| p.skills.notice_factor())),
            };
            let (acts, mut cries) = wolves.update(dt, &watch, |x, z, y| crate::collision::Collider::floor(world, x, z, y + 1.0).or_else(|| world.height_at(x, z)), |from, to| world.blocked(from, to), &mut self.renderer, &self.gpu);
            // What they did: the swings that landed, the spells begun and gone off, and the auras that hold the player.
            let mut effects = Vec::new();
            for act in acts {
                effects.extend(self.foes.handle(act, feet, wolves, &mut self.effects, &mut self.renderer, &self.gpu));
            }
            effects.extend(self.foes.update(dt, feet, wolves, &mut self.effects, &mut self.renderer, &self.gpu));
            cries.extend(self.foes.take_sounds());
            // A bite on the shield rings instead of sounding like teeth in flesh.
            let mut blocked = false;
            if let Some(panels) = &self.panels {
                player.set_block_cost(panels.skills.block_cost_factor());
                player.set_mitigation(panels.stats.mitigation());
            }
            if self.god {
                for r in [&mut player.vitals.health, &mut player.vitals.mana, &mut player.vitals.stamina] {
                    r.current = r.max;
                }
            }
            if safe {
                self.foes.afflictions.clear();
            }
            if let Some(panels) = &mut self.panels {
                panels.set_afflictions(self.foes.afflictions.all());
            }
            for effect in effects.into_iter().filter(|_| !safe) {
                use crate::foes::Effect;
                // What lands as damage, and whether a shield and armor deal with it (blows) or not (magic).
                let hit = match effect {
                    Effect::Hurt { amount, physical } => Some((physical, amount)),
                    Effect::Drain { amount, burn } => {
                        let took = player.drain_mana(amount);
                        (burn && took > 0.0).then_some((false, took))
                    }
                    Effect::Interrupt => {
                        self.caster.interrupt();
                        None
                    }
                    Effect::Knock { from, speed, up } => {
                        self.controller.knock_back(Vec3::new(feet.x - from.x, 0.0, feet.z - from.z).normalize_or_zero(), speed, up);
                        None
                    }
                };
                let Some((physical, amount)) = hit else { continue };
                match if physical { player.take_hit(amount) } else { Some(player.take_magic(amount)) } {
                    None => {
                        blocked = true;
                        if let Some(panels) = &mut self.panels {
                            panels.train(crate::skills::Skill::Blocking, 1.0);
                        }
                    }
                    // The original's combat text shows blocks only with its "Show Damage Reduction" option, off by default.
                    Some(taken) => {
                        if let Some(text) = &mut self.combat_text {
                            text.damage_taken(taken);
                        }
                        // Every kind of armor worn is trained by its share of what is worn (by blows, not by magic).
                        if let (true, Some(panels)) = (physical, &mut self.panels) {
                            for (skill, share) in crate::skills::Skills::armor_shares(&panels.inv.equipment) {
                                panels.train(skill, share);
                            }
                        }
                    }
                }
            }
            if let Some(text) = &mut self.combat_text {
                // "Entering Combat" when a wolf goes for the player, "Leaving Combat" 5 seconds after the last one stops.
                if wolves.engaged() {
                    self.quiet = 0.0;
                    if !std::mem::replace(&mut self.in_combat, true) {
                        log::info!("entering combat: {:?}", wolves.engaged_by(self.camera.pos));
                        text.message("Entering Combat");
                    }
                } else if self.in_combat {
                    self.quiet += dt;
                    if self.quiet >= 5.0 {
                        self.in_combat = false;
                        text.message("Leaving Combat");
                    }
                }
                // "Health Low" once when health falls to a fifth (`COMBAT_TEXT_LOW_HEALTH_THRESHOLD`).
                let low = player.vitals.health.fraction() <= 0.2 && player.vitals.health.current > 0.0;
                if low && !std::mem::replace(&mut self.low_health, true) {
                    text.message("Health Low");
                }
                self.low_health &= low;
            }
            cries.retain(|&(sfx, _)| !(blocked && matches!(sfx, Sfx::Cry(Cry::Bite, _))));
            // The wolves' sounds come from where they are, heard from the camera.
            if let Some(audio) = &mut self.audio {
                audio.set_listener(render_camera.pos, render_camera.yaw);
                for (sfx, at) in cries {
                    audio.play_3d(sfx, world.surface_under(at - Vec3::Y * 0.8) as u32, at);
                }
            }
            wolves.draw_plates(&mut self.renderer.ui, &render_camera, size);
            // Looting: the window closes when the player walks away, and what is left stays on the corpse.
            if let Some(panels) = &mut self.panels {
                let chests = &mut self.chests;
                if panels.looting().and_then(|i| if i >= CHEST_LOOT { chests.as_ref()?.pos(i - CHEST_LOOT) } else { wolves.corpse_pos(i) }).is_some_and(|at| (at - feet).length() > LOOT_LEAVE) {
                    panels.close_loot();
                }
                if panels.talking_to().and_then(|i| wolves.corpse_pos(i)).is_some_and(|at| (at - feet).length() > LOOT_LEAVE) {
                    panels.close_gossip();
                    panels.close_merchant();
                    panels.close_quest();
                }
                if let Some((id, left)) = panels.take_closed_loot() {
                    match chests {
                        Some(chests) if id >= CHEST_LOOT => chests.set_loot(id - CHEST_LOOT, left),
                        _ => wolves.set_loot(id, left),
                    }
                }
                let chest = self.chests.as_ref().and_then(|c| Some(c.lock(c.near(feet, OPEN_RANGE)?).is_some()));
                let pickup = self.pickups.as_ref().is_some_and(|p| p.near(feet, OPEN_RANGE, &panels.quests.wanted(&panels.inv)).is_some());
                let bed = near_bed(&self.beds, &self.world, feet);
                self.hint = match (panels.any_open() || self.lock.is_some(), wolves.lootable_near(feet, LOOT_RANGE), wolves.talkable_near(feet, TALK_RANGE), chest) {
                    (true, ..) => None,
                    (_, Some(_), ..) => Some("Press E to loot"),
                    (_, None, _, Some(true)) => Some("Press E to pick the lock"),
                    (_, None, _, Some(false)) => Some("Press E to open"),
                    (_, None, _, None) if pickup => Some("Press E to pick up"),
                    (_, None, _, None) if bed => Some("Press E to sleep"),
                    (_, None, Some(_), None) => Some("Press E to talk"),
                    _ => None,
                };
                // Creatures that died to the sword count for the kill goals of quests.
                panels.on_kills(&wolves.take_deaths());
                wolves.set_wanted(panels.quests.wanted(&panels.inv));
                panels.update_quests();
                if self.names {
                    panels.draw_labels(&mut self.renderer.ui, &self.gpu, size, &wolves.labels(&render_camera, size));
                }
            }
            if let Some(text) = &mut self.combat_text {
                text.update(dt);
                text.draw(&mut self.renderer.ui, &render_camera, size);
            }
            // Dead: back at the spawn point with everything restored.
            if player.vitals.health.current <= 0.0 {
                log::info!("you died");
                player.revive();
                self.foes.clear(&mut self.effects);
                if let Some(panels) = &mut self.panels {
                    panels.consumed.clear();
                    panels.refresh();
                }
                (self.camera.pos.x, self.camera.pos.z) = self.spawn;
                self.snap_to_ground = Some(0.0);
                self.controller.reset();
            }
        }
        if let Some(chests) = &mut self.chests {
            let world = &self.world;
            chests.update(dt, |x, z| world.height_at(x, z), &mut self.renderer, &self.gpu);
        }
        if std::mem::take(&mut self.pickups_stale) {
            self.load_pickups();
        }
        if let Some(pickups) = &mut self.pickups {
            let world = &self.world;
            pickups.update(dt, |x, z| world.height_at(x, z), &mut self.renderer);
        }
        self.step_lock(dt);
        (self.pointer, self.hover) = {
            let (kind, hover) = self.cursor_context(&render_camera, size);
            (kind, hover)
        };
        if let Some(panels) = &mut self.panels {
            let feet = Mat4::from_translation(self.camera.pos - Vec3::Y * EYE_HEIGHT);
            if panels.take_level_up() {
                self.levelfx = self.effects.spawn(&mut self.renderer, &self.gpu, crate::effects::LEVEL_UP, false, feet);
            }
            if let Some(id) = self.levelfx {
                self.effects.set_world(id, feet);
                if !self.effects.active(id) {
                    self.levelfx = None;
                }
            }
        }
        // Slept: the clock is at the hour chosen, and health, mana and stamina are full.
        if let Some(hour) = self.panels.as_mut().and_then(|p| p.take_sleep()) {
            self.hour = hour;
            if let Some(player) = &mut self.player {
                let v = &mut player.vitals;
                for r in [&mut v.health, &mut v.mana, &mut v.stamina] {
                    r.current = r.max;
                }
            }
            if let Some(panels) = &mut self.panels {
                panels.notice("You wake up rested.");
            }
            self.set_grab(true);
        }
        // A place picked on the world map: go there and hand the mouse back to the game.
        if let Some((north, west, height)) = self.panels.as_mut().and_then(|p| p.take_travel()) {
            self.teleport(north, west, height);
            self.set_grab(true);
        }
        // What using items began goes on: food and drink restore, elixirs run out, the way home counts down.
        if let (Some(panels), Some(player)) = (&mut self.panels, &mut self.player) {
            panels.set_calm(!self.moving && !self.in_combat && !player.is_striking());
            panels.set_mounted(player.is_mounted());
            if panels.mounting() && self.world.indoors(self.camera.pos - Vec3::Y * (EYE_HEIGHT - 1.0)) {
                panels.cancel_mount("You can't mount here.");
            }
            let way_home = panels.tick_items(dt);
            if let Some((display, percent)) = panels.take_steed() {
                let inside = self.world.indoors(self.camera.pos - Vec3::Y * (EYE_HEIGHT - 1.0));
                if self.in_combat && !player.is_mounted() {
                    panels.error("You can't do that while in combat.");
                } else if inside && !player.is_mounted() {
                    panels.error("You can't mount here.");
                } else if let Err(e) = player.ride(display, percent, &mut self.renderer, &self.gpu) {
                    log::warn!("cannot ride: {e:#}");
                    panels.error("You can't mount that.");
                }
            }
            // A rider who goes indoors is put down: no horse goes through a door.
            self.indoor_check -= dt;
            if player.is_mounted() && self.indoor_check <= 0.0 {
                self.indoor_check = 0.25;
                if self.world.indoors(self.camera.pos - Vec3::Y * (EYE_HEIGHT - 1.0)) {
                    player.dismount();
                    panels.error("You can't mount here.");
                }
            }
            let (heal, mana) = panels.take_restores();
            player.heal(heal);
            let mana_bar = &mut player.vitals.mana;
            mana_bar.current = (mana_bar.current + mana).min(mana_bar.max);
            if way_home == Some(crate::consume::WayHome::Arrived) {
                (self.camera.pos.x, self.camera.pos.z) = self.spawn;
                self.snap_to_ground = Some(0.0);
            }
        }
        if let (Some(panels), Some(player)) = (&mut self.panels, &mut self.player) {
            if panels.take_equipment_change() {
                player.set_equipment(&mut self.renderer, &self.gpu, &panels.inv.equipment, &panels.stats);
            }
            panels.render_pictures(&mut self.renderer, &self.gpu, player, &self.world);
            if let Some(wolves) = &self.wolves {
                panels.render_target(&mut self.renderer, &self.gpu, wolves, &self.world);
            }
        }
        if let (Some(bar), Some(player)) = (&self.actionbar, &self.player) {
            let cooldowns = self.panels.as_ref().map_or(&[][..], |p| &p.cooldowns[..]);
            let items = |i| self.panels.as_ref().map_or((0, 0.0), |p| p.item_state(i));
            bar.draw(&mut self.renderer.ui, size, player, self.panels.as_ref().is_some_and(|p| p.character_open()), self.panels.as_ref().map_or(crate::levels::Progress::new(), |p| p.progress), cooldowns, &items);
        }
        let mut discovered = false;
        if let Some(panels) = &mut self.panels {
            panels.draw(&mut self.renderer.ui, &self.gpu, size);
            panels.draw_overlay(&mut self.renderer.ui, &self.gpu, size, dt, self.hint);
            panels.set_quest_targets(self.wolves.as_ref(), self.chests.as_ref(), self.pickups.as_ref());
            panels.draw_map(&mut self.renderer.ui, &self.gpu, size, self.camera.pos, self.camera.yaw);
            if let Some(bar) = &self.actionbar {
                bar.draw_drag(&mut self.renderer.ui, size);
            }
            if let Some((_, lock)) = &self.lock {
                panels.draw_lockpick(&mut self.renderer.ui, &self.gpu, size, lock, self.lock_art.as_ref(), self.lock_time);
            }
            if self.player.as_ref().is_some_and(|p| p.is_sneaking()) {
                panels.draw_sneak(&mut self.renderer.ui, &self.gpu, size);
            }
            if self.lock.is_none() {
                discovered = panels.discover(self.camera.pos - Vec3::Y * EYE_HEIGHT);
                discovered |= panels.explore(self.world.area_at(self.camera.pos.x, self.camera.pos.z));
                panels.draw_compass(&mut self.renderer.ui, &self.gpu, size, self.camera.pos, self.camera.yaw);
            }
            if self.grabbed && self.lock.is_none() {
                let alert = self.player.as_ref().is_some_and(|p| p.is_sneaking()).then(|| self.wolves.as_ref().map_or(0.0, |w| w.alert(self.camera.pos - Vec3::Y * EYE_HEIGHT)));
                if let Some(cursors) = &self.cursors {
                    cursors.reticle(&mut self.renderer.ui, size, alert);
                }
            }
            // The editor, over the windows and under the pointer.
            let hover = self.cursor.filter(|_| self.editor.as_ref().is_some_and(|e| e.placing())).and_then(|p| {
                let (origin, dir) = self.eye?.ray(glam::Vec2::new(p.0, p.1));
                self.world.ray_hit(origin, origin + dir * 400.0)
            });
            if let Some(editor) = &mut self.editor {
                let [north, west, eye] = render_to_wow(self.camera.pos);
                editor.set_hover(hover);
                for action in editor.draw(&mut self.renderer.ui, size, crate::editor::Place { north, west, height: eye - EYE_HEIGHT, yaw: self.camera.yaw.to_degrees().rem_euclid(360.0) }, crate::propedit::Eye::of(&render_camera, size), &|x, z| self.world.height_at(x, z)) {
                    match action {
                        crate::editor::Action::Teleport(north, west) => match &mut self.free {
                            // The camera goes to look at the place from a little way off and above.
                            Some(free) => {
                                let target = Vec3::new(-west, self.world.height_at(-west, -north).unwrap_or(free.camera.pos.y), -north);
                                let (sy, cy) = free.camera.yaw.sin_cos();
                                free.camera.pos = target - Vec3::new(sy, 0.0, -cy) * 16.0 + Vec3::Y * 9.0;
                                free.camera.pitch = -0.45;
                            }
                            None => {
                                (self.camera.pos.x, self.camera.pos.z) = (-west, -north);
                                self.snap_to_ground = Some(0.0);
                                self.controller.reset();
                            }
                        },
                        crate::editor::Action::Apply => self.apply = true,
                        crate::editor::Action::Props(props) => {
                            // Those to pick up are drawn by `pickups`, made again at the start of the next frame when they changed.
                            let pickups: Vec<content::Prop> = props.iter().filter(|p| p.pickup.is_some()).cloned().collect();
                            if pickups != self.pickup_props {
                                (self.pickup_props, self.pickups_stale) = (pickups, true);
                            }
                            self.beds = bed_spots(&props, &self.map);
                            self.world.set_props(props);
                        }
                    }
                }
                editor.render_preview(&mut self.renderer, &self.gpu, &self.world, self.camera.pos, dt);
                editor.render_gallery(&mut self.renderer, &self.gpu, &self.world, self.camera.pos, false);
                editor.update_ghosts(&mut self.renderer, &self.gpu, self.world.refreshing(), dt);
            }
            if let Some(console) = &self.console {
                console.draw(&mut self.renderer.ui, size);
            }
            // The pointer, over everything; what the mouse carries is drawn instead of it.
            if let (Some(cursors), Some(pos), false) = (&self.cursors, self.cursor, panels.carrying()) {
                cursors.draw(&mut self.renderer.ui, self.pointer, pos);
            }
            if let Some(audio) = &mut self.audio {
                panels.take_events().into_iter().for_each(|sfx| audio.play(sfx, 0));
                if let Some(sound) = panels.take_sound() {
                    audio.apply(&sound);
                }
            }
            if let Some(i) = panels.take_interface() {
                (self.controller.look_scale, self.controller.invert_y) = (i.look_scale(), i.invert_mouse);
                if let Some(free) = &mut self.free {
                    (free.controller.look_scale, free.controller.invert_y) = (i.look_scale(), i.invert_mouse);
                }
                if let Some(p) = &mut self.player {
                    p.follow = i.follow;
                }
                self.names = i.npc_names;
            }
            if let Some(v) = panels.take_video() {
                self.weather.detail = v.weather_scale();
                self.effects.detail = v.particle_scale();
                (self.renderer.gamma, self.renderer.prop_reach, self.renderer.shadows) = (v.gamma, v.prop_reach(), v.shadows);
                crate::uixml::set_ui_scale(v.interface_scale());
                v.apply_window(&self.window);
                // The surface is set up again once this frame is shown (it cannot be while the frame is held).
                if self.config.present_mode != v.present_mode() {
                    self.config.present_mode = v.present_mode();
                    self.reconfigure = true;
                }
                if let Some(sky) = &mut self.sky {
                    sky.set_view_distance(v.distance());
                }
            }
            match panels.take_menu_action() {
                Some(crate::panels::MenuAction::Logout) => (self.logout, self.restart) = (true, true),
                Some(crate::panels::MenuAction::Quit) => self.quit = true,
                None => {}
            }
        }
        if discovered {
            self.save_game();
        }
        let drawing = Instant::now();
        self.renderer.prepare_weather(&self.gpu, &self.weather, &render_camera, (self.config.width, self.config.height));
        self.renderer.render(
            &self.gpu,
            &view,
            (self.config.width, self.config.height),
            &render_camera,
            self.start.elapsed().as_secs_f32(),
            self.world.tiles(),
        );
        self.render_time += drawing.elapsed();
        self.window.pre_present_notify();
        frame.present();
        if std::mem::take(&mut self.reconfigure) {
            self.surface.configure(&self.gpu.device, &self.config);
        }

        self.frames += 1;
        if self.title_timer.elapsed() >= Duration::from_millis(500) {
            let fps = self.frames as f32 / self.title_timer.elapsed().as_secs_f32();
            let render_ms = self.render_time.as_secs_f32() * 1000.0 / self.frames.max(1) as f32;
            log::info!("{fps:.0} fps, render {render_ms:.2} ms on the CPU");
            self.render_time = Duration::ZERO;
            let [wx, wy, wz] = render_to_wow(self.camera.pos);
            let area = self
                .world
                .area_at(self.camera.pos.x, self.camera.pos.z)
                .map(|a| self.world.areas.display(a))
                .unwrap_or_else(|| "...".into());
            let loading = if self.world.pending_count() > 0 {
                format!(" | loading {} tiles", self.world.pending_count())
            } else {
                String::new()
            };
            self.window.set_title(&format!(
                "rsworld | {area} | ({wx:.0}, {wy:.0}, {wz:.0}) | {fps:.0} fps ({render_ms:.1} ms draw) | {:?} | {}{loading}",
                self.controller.mode,
                match self.player.as_ref().map(|p| p.view) {
                    Some(View::Third) => "third person",
                    Some(View::First) => "first person",
                    None => "no player",
                },
            ));
            self.frames = 0;
            self.title_timer = Instant::now();
        }
    }

    /// Open or close the character sheet or the backpack; the mouse is free while one is open.
    fn toggle_panel(&mut self, character: bool) {
        let Some(panels) = &mut self.panels else { return };
        if character {
            panels.toggle_character_tab(false);
        } else {
            panels.toggle_backpack();
        }
        let open = panels.any_open();
        self.set_grab(!open);
    }

    /// The interact key: loot the nearest corpse in reach, or else talk to the nearest friendly creature. The mouse
    /// is free while a window is open.
    fn interact(&mut self) {
        if self.panels.as_ref().is_none_or(|p| p.any_open()) || self.lock.is_some() {
            return;
        }
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let corpse = self.wolves.as_ref().and_then(|w| w.lootable_near(feet, LOOT_RANGE));
        let chest = self.chests.as_ref().and_then(|c| c.near(feet, OPEN_RANGE));
        let pickup = self.pickups.as_ref().zip(self.panels.as_ref()).and_then(|(p, panels)| p.near(feet, OPEN_RANGE, &panels.quests.wanted(&panels.inv)));
        let talk = self.wolves.as_ref().and_then(|w| w.talkable_near(feet, TALK_RANGE));
        if corpse.is_none() && chest.is_none() && pickup.is_none() && near_bed(&self.beds, &self.world, feet) {
            if let Some(panels) = &mut self.panels {
                panels.ask_sleep();
            }
            self.set_grab(false);
            return;
        }
        if let Some(i) = corpse {
            self.loot_corpse(i);
        } else if let Some(i) = chest {
            self.use_chest(i);
        } else if let Some(i) = pickup {
            self.use_pickup(i);
        } else if let Some(i) = talk {
            self.talk_to(i);
        }
    }

    /// Open the loot window of corpse `i`.
    fn loot_corpse(&mut self, i: usize) {
        let (Some(wolves), Some(panels)) = (&self.wolves, &mut self.panels) else { return };
        if let Some(loot) = wolves.loot(i).cloned() {
            panels.open_loot(i, loot);
            self.set_grab(false);
        }
    }

    /// Talk to creature `i` (which turns to face the one who speaks to him), if it has something to say.
    fn talk_to(&mut self, i: usize) {
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let (Some(wolves), Some(panels)) = (&mut self.wolves, &mut self.panels) else { return };
        let Some(def) = wolves.def(i).filter(|d| d.gossip.is_some()) else { return };
        wolves.face(i, feet);
        wolves.set_target(i);
        panels.open_gossip(i, def);
        self.set_grab(false);
    }

    /// Pick up prop `i`: its item goes into the bag (the quest that wants it counts it), and the prop is gone for a while.
    fn use_pickup(&mut self, i: usize) {
        let (Some(pickups), Some(panels)) = (&mut self.pickups, &mut self.panels) else { return };
        if let Some(item) = pickups.take(i) {
            panels.inv.add(crate::items::Stack { item, count: 1 });
            let text = panels.string("LOOT_ITEM_PUSHED_SELF").replacen("%s", crate::items::get(item).name, 1);
            panels.notice(&text);
            panels.sound("ChestOpen");
        }
    }

    /// Use chest `i`: a locked one is picked (on a sweet spot nobody tells) with a lockpick, one that is not opens at once into the
    /// loot window.
    fn use_chest(&mut self, i: usize) {
        let (Some(chests), Some(panels)) = (&mut self.chests, &mut self.panels) else { return };
        if let Some(level) = chests.lock(i) {
            if panels.picks() == 0 {
                panels.error("This chest is locked. You need a lockpick.");
                return;
            }
            let sweet = (self.start.elapsed().subsec_nanos() % 1400) as f32 / 10.0 - 70.0;
            let mut lock = Lockpick::new(level, sweet);
            lock.window_factor = panels.skills.lock_window_factor();
            self.lock = Some((i, lock));
            (self.lock_turn, self.lock_time, self.input) = (false, 0.0, Input::default());
            return;
        }
        panels.sound("ChestOpen");
        if let Some(loot) = chests.open(i, &panels.quests.wanted(&panels.inv)) {
            panels.open_loot(CHEST_LOOT + i, loot);
            self.set_grab(false);
        }
    }

    /// The pointer for what is under it now, and what that is (for a click): over a window, or over a creature or chest in the world.
    fn cursor_context(&mut self, camera: &Camera, size: (u32, u32)) -> (Kind, Option<Hover>) {
        let Some(pos) = self.cursor else { return (Kind::Point, None) };
        if let Some(kind) = self.panels.as_mut().and_then(|p| p.cursor_at(pos, size)) {
            return (kind, None);
        }
        if let Some((i, kind)) = self.wolves.as_ref().and_then(|w| w.pick(camera, size, pos)) {
            return (kind, Some(Hover::Creature(i)));
        }
        if let Some((i, kind)) = self.chests.as_ref().and_then(|c| c.pick(camera, size, pos)) {
            return (kind, Some(Hover::Chest(i)));
        }
        if let Some(i) = self.pickups.as_ref().zip(self.panels.as_ref()).and_then(|(p, panels)| p.pick(camera, size, pos, &panels.quests.wanted(&panels.inv))) {
            return (Kind::Interact, Some(Hover::Pickup(i)));
        }
        (Kind::Point, None)
    }

    /// A click in the world with the mouse free: the left button selects the creature under the pointer, the right one uses it
    /// (talk, loot, open) if it is near enough. False when the click was on nothing.
    fn world_click(&mut self, right: bool) -> bool {
        let Some(hover) = self.hover else { return false };
        if !right {
            if let (Hover::Creature(i), Some(wolves)) = (hover, &mut self.wolves) {
                wolves.set_target(i);
                return true;
            }
            return false;
        }
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let at = match hover {
            Hover::Creature(i) => self.wolves.as_ref().and_then(|w| w.corpse_pos(i)),
            Hover::Chest(i) => self.chests.as_ref().and_then(|c| c.pos(i)),
            Hover::Pickup(i) => self.pickups.as_ref().and_then(|p| p.pos(i)),
        };
        let Some(at) = at else { return false };
        let distance = Vec3::new(at.x - feet.x, 0.0, at.z - feet.z).length();
        let (range, far) = match hover {
            Hover::Creature(i) if self.wolves.as_ref().is_some_and(|w| w.loot(i).is_some()) => (LOOT_RANGE, "ERR_LOOT_TOO_FAR"),
            Hover::Creature(_) => (TALK_RANGE, "ERR_USE_TOO_FAR"),
            Hover::Chest(_) | Hover::Pickup(_) => (OPEN_RANGE, "ERR_USE_TOO_FAR"),
        };
        if self.panels.as_ref().is_none_or(|p| p.any_open()) || self.lock.is_some() {
            return true;
        }
        if distance > range {
            if let Some(p) = &mut self.panels {
                let text = p.string(far);
                p.error(&text);
            }
            return true;
        }
        match hover {
            Hover::Creature(i) if range == LOOT_RANGE => self.loot_corpse(i),
            Hover::Creature(i) => self.talk_to(i),
            Hover::Chest(i) => self.use_chest(i),
            Hover::Pickup(i) => self.use_pickup(i),
        }
        true
    }

    /// Stop picking the lock.
    fn end_lock(&mut self) {
        (self.lock, self.lock_turn) = (None, false);
    }

    /// One step of lockpicking: the lock turns or falls back; a pick that breaks is gone (and with the last one the picking
    /// ends); a lock that turns all the way opens the chest and shows what is in it.
    fn step_lock(&mut self, dt: f32) {
        let Some((chest, lock)) = &mut self.lock else { return };
        self.lock_time += dt;
        let (chest, event) = (*chest, lock.update(dt, self.lock_turn));
        let (Some(panels), Some(chests)) = (&mut self.panels, &mut self.chests) else { return };
        match event {
            Some(Event::Broke) => {
                panels.train(crate::skills::Skill::Lockpicking, 1.0);
                panels.sound("Item - Lockpick");
                panels.use_pick();
                if panels.picks() == 0 {
                    panels.error("Your last lockpick broke.");
                    self.end_lock();
                } else {
                    panels.notice("Your lockpick broke.");
                }
            }
            Some(Event::Opened) => {
                panels.train(crate::skills::Skill::Lockpicking, 3.0);
                panels.sound("ChestOpen");
                if let Some(loot) = chests.open(chest, &panels.quests.wanted(&panels.inv)) {
                    panels.open_loot(CHEST_LOOT + chest, loot);
                    self.end_lock();
                    self.set_grab(false);
                } else {
                    self.end_lock();
                }
            }
            None => {}
        }
    }

    /// A mouse button on the open panels: true when they took it (the game must not act on it).
    fn panel_mouse(&mut self, down: bool, left: bool) -> bool {
        let (Some(panels), Some(pos)) = (&mut self.panels, self.cursor) else { return false };
        let size = (self.config.width, self.config.height);
        if !panels.any_open() {
            return false;
        }
        if down {
            panels.is_over(pos, size) && panels.mouse_down(pos, size)
        } else {
            panels.mouse_up(pos, size, left);
            true
        }
    }

    /// Open or close the console; the mouse is free while it is open and the keys held down are let go.
    fn set_console(&mut self, open: bool) {
        let Some(console) = &mut self.console else { return };
        console.open = open;
        if open {
            self.input = Input::default();
            self.set_grab(false);
        } else if !self.panels.as_ref().is_some_and(|p| p.any_open()) {
            self.set_grab(true);
        }
    }

    /// Fly (F, `tcl`): nothing stops the character, and back on foot it lands on the terrain.
    fn toggle_fly(&mut self) {
        self.controller.toggle_mode();
        if let Some(p) = &mut self.player {
            p.set_sneaking(false);
        }
        if self.controller.mode == Mode::Walk {
            if let Some(g) = self.world.height_at(self.camera.pos.x, self.camera.pos.z) {
                self.camera.pos.y = self.camera.pos.y.max(g + EYE_HEIGHT);
            }
        }
    }

    /// Run a line of the console and print the answer.
    fn run_line(&mut self, line: &str) {
        if line.trim().eq_ignore_ascii_case("clear") {
            return self.console.iter_mut().for_each(|c| c.clear());
        }
        let result = crate::console::parse(line).and_then(|command| self.run_command(command));
        if let Some(console) = &mut self.console {
            console.reply(result);
        }
    }

    fn run_command(&mut self, command: Command) -> Result<String, String> {
        let (Some(panels), Some(player)) = (&mut self.panels, &mut self.player) else { return Err("the game is not ready".into()) };
        let content = content::get();
        Ok(match command {
            Command::Help => crate::console::Console::help(),
            Command::Logout => {
                (self.logout, self.restart) = (true, true);
                "logging out".into()
            }
            Command::GodMode => {
                self.god = !self.god;
                format!("god mode {}", if self.god { "on" } else { "off" })
            }
            Command::Collision => {
                self.toggle_fly();
                format!("collision {}", if self.controller.mode == Mode::Walk { "on" } else { "off (flying)" })
            }
            Command::FreeCamera => {
                self.detached = !self.detached;
                format!("free camera {}", if self.detached { "on" } else { "off" })
            }
            Command::Level(n) => {
                panels.set_level(n);
                format!("level {n}")
            }
            Command::Xp(n) => {
                panels.gain_xp(n);
                format!("gained {n} experience")
            }
            Command::Gold(n) => {
                panels.inv.money = (panels.inv.money as i64 + n * 10_000).clamp(0, u32::MAX as i64) as u32;
                panels.refresh();
                format!("{} gold, {} silver, {} copper", panels.inv.money / 10_000, panels.inv.money / 100 % 100, panels.inv.money % 100)
            }
            Command::Skill(skill, rank) => {
                let all = skills::ALL;
                let chosen = skill.as_slice().iter().chain(skill.is_none().then_some(&all[..]).into_iter().flatten());
                chosen.for_each(|&s| panels.skills.set(s, rank));
                panels.refresh();
                format!("{} set to {rank}", skill.map_or("every skill", |s| s.name()))
            }
            Command::Item(item, count) => {
                let def = crate::items::get(item);
                if !panels.inv.add(crate::items::Stack { item, count }) {
                    return Err("no room in the backpack".into());
                }
                panels.refresh();
                format!("{count} x {}", def.name)
            }
            Command::Spell(spell) => {
                let all: Vec<usize> = (0..content.spells.len()).collect();
                let learned = spell.as_slice().iter().chain(spell.is_none().then_some(&all[..]).into_iter().flatten()).filter(|&&s| panels.learn(s)).count();
                format!("learned {learned} spells")
            }
            Command::Teleport(north, west) => {
                self.teleport(north, west, None);
                format!("teleported to {north:.1}, {west:.1}")
            }
            Command::Heal => {
                for r in [&mut player.vitals.health, &mut player.vitals.mana, &mut player.vitals.stamina] {
                    r.current = r.max;
                }
                "healed".into()
            }
            Command::Time(hour) => {
                self.fixed_time = hour.is_some();
                self.hour = hour.unwrap_or(self.hour);
                format!("it is {:02}:{:02}{}", self.hour as u32, (self.hour.fract() * 60.0) as u32, if self.fixed_time { "" } else { ", and time runs" })
            }
            Command::Pos => {
                let [north, west, _] = render_to_wow(self.camera.pos);
                format!("north {north:.1}, west {west:.1}, height {:.1}", self.camera.pos.y)
            }
        })
    }

    /// Put the player at a place (north, west) and on the ground there once the terrain has streamed in; with a feet `height`
    /// on the floor at that height (a building, a bridge), as a loaded save does.
    fn teleport(&mut self, north: f32, west: f32, height: Option<f32>) {
        (self.camera.pos.x, self.camera.pos.z) = (-west, -north);
        self.snap_to_ground = Some(0.0);
        self.saved_feet = height;
        self.controller.reset();
        // The loading screen is up until the terrain there has streamed in (at least a moment, for a place close by).
        if let (None, Some(mut loading)) = (&self.loading, self.spare_loading.take()) {
            loading.restart(1.0);
            self.loading = Some(loading);
        }
    }

    fn key(&mut self, code: KeyCode, pressed: bool, event_loop: &ActiveEventLoop) {
        log::debug!("key {code:?} pressed={pressed}");
        // F1 opens and closes the editor.
        if let (KeyCode::F1, true, Some(editor)) = (code, pressed, &mut self.editor) {
            editor.toggle();
            let open = editor.open;
            self.looking = false;
            self.rotating = false;
            if open {
                self.set_grab(false);
            } else if !self.panels.as_ref().is_some_and(|p| p.any_open()) {
                self.set_grab(true);
            }
            return;
        }
        // F2 opens and closes the console; the mouse is free while it is open and the keys held down are let go.
        if let (KeyCode::F2, true, Some(console)) = (code, pressed, &self.console) {
            self.set_console(!console.open);
            return;
        }
        // F5 quicksaves into a second file beside the character's; F9 puts it back as the save and starts the game again from it.
        if pressed && matches!(code, KeyCode::F5 | KeyCode::F9) && self.free.is_none() {
            if let Some(path) = self.save_path.clone() {
                let quick = path.with_extension("quick");
                let note = if code == KeyCode::F5 {
                    match self.capture().map(|saved| save::write(&quick, &saved)) {
                        Some(Ok(())) => "Quicksaved",
                        _ => "Cannot quicksave",
                    }
                } else {
                    match save::read(&quick).and_then(|q| q.map(|q| save::write(&path, &q)).transpose()) {
                        // Nothing may save over the loaded game while the game starts again.
                        Ok(Some(())) => {
                            (self.save_path, self.restart) = (None, true);
                            "Quickloading"
                        }
                        Ok(None) => "No quicksave",
                        Err(e) => {
                            log::warn!("cannot quickload: {e:#}");
                            "Cannot quickload"
                        }
                    }
                };
                if let Some(panels) = &mut self.panels {
                    panels.notice(note);
                }
            }
            return;
        }
        // With the editor open: G drops what is selected (or put down) to the ground, Ctrl + S saves and applies.
        if let (true, true, Some(editor)) = (pressed, self.free.is_some(), &mut self.editor) {
            match code {
                KeyCode::KeyG => {
                    let world = &self.world;
                    editor.snap_to_ground(|x, z| world.height_at(x, z));
                    return;
                }
                KeyCode::KeyS if self.ctrl => {
                    editor.request_save();
                    return;
                }
                _ => {}
            }
        }
        // With the editor's camera out the keys fly it; the game's own keys are not heard.
        if self.free.is_some() && !matches!(code, KeyCode::KeyW | KeyCode::KeyA | KeyCode::KeyS | KeyCode::KeyD | KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Space | KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::ShiftLeft | KeyCode::ShiftRight) {
            return;
        }
        // While a lock is picked the keys turn the lock, and Esc leaves; nothing else is heard.
        if self.lock.is_some() {
            match code {
                KeyCode::Escape if pressed => self.end_lock(),
                KeyCode::KeyA | KeyCode::KeyD | KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Space => self.lock_turn = pressed,
                _ => {}
            }
            return;
        }
        if let (Some(slot), Some(bar), Some(player)) = (slot_of_key(code), &mut self.actionbar, &mut self.player) {
            if pressed {
                self.cast_request = bar.press(slot, player).or(self.cast_request);
            } else {
                bar.release(slot, player);
            }
            return;
        }
        match code {
            KeyCode::KeyW | KeyCode::ArrowUp => self.input.forward = pressed,
            KeyCode::KeyS | KeyCode::ArrowDown => self.input.back = pressed,
            KeyCode::KeyA | KeyCode::ArrowLeft => self.input.left = pressed,
            KeyCode::KeyD | KeyCode::ArrowRight => self.input.right = pressed,
            KeyCode::Space => {
                self.input.up = pressed;
                self.input.jump_pressed |= pressed;
            }
            KeyCode::ControlLeft | KeyCode::ControlRight => {
                // Flying down, or on foot: sneaking on and off.
                self.input.down = pressed;
                if let (true, Mode::Walk, None, Some(p)) = (pressed, self.controller.mode, &self.free, &mut self.player) {
                    p.toggle_sneak();
                }
            }
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.input.fast = pressed,
            KeyCode::KeyE if pressed => self.interact(),
            KeyCode::Tab if pressed => {
                // The nearest enemy ahead; again, the next.
                let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
                if let Some(wolves) = &mut self.wolves {
                    wolves.cycle_target(feet, self.camera.yaw);
                }
            }
            KeyCode::KeyC if pressed => self.toggle_panel(true),
            KeyCode::KeyB if pressed => self.toggle_panel(false),
            KeyCode::KeyK if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_character_tab(true);
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyP if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_spellbook();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyM if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_map();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyL if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_quest_log();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyR if pressed => {
                if let Some(p) = &mut self.player {
                    p.toggle_sheath();
                }
            }
            KeyCode::KeyV if pressed => {
                if let Some(p) = &mut self.player {
                    p.toggle_view();
                }
            }
            // Stand-in for an enemy: a 10 damage hit, blocked (at a stamina cost) if the shield is up.
            KeyCode::KeyH if pressed => {
                if let Some(p) = &mut self.player {
                    match p.take_hit(10.0) {
                        None => log::info!("test hit: blocked"),
                        Some(n) => log::info!("test hit: took {n} damage"),
                    }
                }
            }
            KeyCode::KeyF if pressed => self.toggle_fly(),
            KeyCode::Escape if pressed => {
                if self.panels.as_ref().is_some_and(|p| p.any_open()) {
                    let panels = self.panels.as_mut().unwrap();
                    panels.escape();
                    // The mouse goes back to the game once nothing is open.
                    if !panels.any_open() {
                        self.set_grab(true);
                    }
                } else if self.wolves.as_ref().is_some_and(|w| w.target().is_some()) {
                    // The first Esc lets go of the target.
                    self.wolves.as_mut().unwrap().clear_target();
                } else if let Some(panels) = &mut self.panels {
                    // Nothing else to close: the game menu.
                    panels.open_menu();
                    self.set_grab(false);
                } else if self.grabbed {
                    self.set_grab(false);
                } else {
                    event_loop.exit();
                }
            }
            _ => {}
        }
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
        let Some(state) = self.state.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(KeyCode::ControlLeft | KeyCode::ControlRight) = event.physical_key {
                    state.ctrl = event.state == ElementState::Pressed;
                }
                // The console takes everything that is typed (F2 is heard below, to close it).
                if let Some(console) = state.console.as_mut().filter(|c| c.open) {
                    use crate::console::Key;
                    use winit::keyboard::{Key as Logical, NamedKey};
                    if event.physical_key != PhysicalKey::Code(KeyCode::F2) {
                        if event.state == ElementState::Pressed {
                            if let Some(text) = &event.text {
                                console.typed(text);
                            }
                            let line = match event.logical_key {
                                Logical::Named(NamedKey::Backspace) => console.key(Key::Backspace),
                                Logical::Named(NamedKey::Enter) => console.key(Key::Enter),
                                Logical::Named(NamedKey::Tab) => console.key(Key::Tab),
                                Logical::Named(NamedKey::ArrowUp) => console.key(Key::Up),
                                Logical::Named(NamedKey::ArrowDown) => console.key(Key::Down),
                                Logical::Named(NamedKey::Escape) => {
                                    state.set_console(false);
                                    None
                                }
                                _ => None,
                            };
                            if let Some(line) = line {
                                state.run_line(&line);
                            }
                        }
                        return;
                    }
                }
                // What is typed goes to the field of the editor that is being edited, and nothing else hears it.
                if let Some(editor) = state.editor.as_mut().filter(|e| e.open) {
                    if event.state == ElementState::Pressed {
                        if let Some(text) = &event.text {
                            editor.typed(text);
                        }
                        match event.logical_key {
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => editor.key(crate::editor::Key::Backspace),
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => editor.key(crate::editor::Key::Enter),
                            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => editor.key(crate::editor::Key::Escape),
                            _ => {}
                        }
                    }
                    // (F1 is not typed: it always gets to close the editor.)
                    // Ctrl + S saves even in a field: what is typed is kept first.
                    let save_chord = state.ctrl && event.state == ElementState::Pressed && event.physical_key == PhysicalKey::Code(KeyCode::KeyS);
                    if save_chord && editor.typing() {
                        editor.key(crate::editor::Key::Enter);
                    }
                    if editor.typing() && event.physical_key != PhysicalKey::Code(KeyCode::F1) && !save_chord {
                        return;
                    }
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if !event.repeat {
                        state.key(code, event.state == ElementState::Pressed, event_loop);
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let (Some(editor), Some(pos)) = (state.editor.as_mut().filter(|e| e.open), state.cursor) {
                    if editor.over(pos, state.config.height) {
                        editor.click();
                        return;
                    }
                    // A click in the world is a click away from the field being typed in: it keeps what is typed and lets the keys go.
                    editor.click();
                    // A click on the gizmo takes hold of it; a click on a prop selects it.
                    let world = &state.world;
                    if editor.world_press(&|x, z| world.height_at(x, z)) {
                        return;
                    }
                }
                // Holding the mouse to look round, a click does not swing the sword.
                if state.looking {
                    return;
                }
                // Putting props down: the click stands the chosen model where the pointer touches the world.
                if let Some(pos) = state.cursor.filter(|_| state.editor.as_ref().is_some_and(|e| e.placing())) {
                    if let Some(hit) = state.cursor_hit(pos) {
                        if let Some(editor) = &mut state.editor {
                            editor.place(hit);
                        }
                    }
                    return;
                }
                if state.lock.is_some() {
                    state.lock_turn = true;
                } else if state.panel_mouse(true, true) {
                } else if !state.grabbed {
                    // A click on an action button uses it; anywhere else it hands the mouse back to the game.
                    let size = (state.config.width, state.config.height);
                    let slot = state.actionbar.as_ref().and(state.cursor).and_then(|pos| ActionBar::slot_at(pos, size));
                    // An item picked up with a click is put on the button clicked.
                    if let (Some(slot), Some(bar), Some(panels)) = (slot, &mut state.actionbar, &mut state.panels) {
                        if let Some(action) = panels.take_carried_action() {
                            bar.place(slot, action);
                            return;
                        }
                    }
                    match (slot, &mut state.actionbar, &mut state.player) {
                        // Shift lifts the action off its button to move or drop it instead of using it.
                        (Some(slot), Some(bar), Some(_)) if state.input.fast => bar.pick_up(slot),
                        (Some(slot), Some(bar), Some(player)) => {
                            state.cast_request = bar.press(slot, player).or(state.cast_request);
                            state.mouse_slot = Some(slot);
                        }
                        _ if state.panels.as_ref().is_some_and(|p| p.any_open()) => {}
                        // A click on a creature selects it; a click on nothing hands the mouse back to the game.
                        _ => {
                            if !state.world_click(false) {
                                state.set_grab(true);
                            }
                        }
                    }
                } else if let Some(p) = &mut state.player {
                    p.attack_press();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                state.lock_turn = false;
                if let Some(editor) = &mut state.editor {
                    editor.release();
                }
                if let Some(p) = &mut state.player {
                    p.attack_release();
                }
                // An action dragged from the spellbook or off the bar lands on the button under the pointer; dropped anywhere
                // else it is gone from the bar.
                if let Some(bar) = &mut state.actionbar {
                    let from_book = state.panels.as_mut().and_then(|p| p.take_drag());
                    if let Some(action) = bar.take_drag().or(from_book) {
                        let size = (state.config.width, state.config.height);
                        if let Some(slot) = state.cursor.and_then(|pos| ActionBar::slot_at(pos, size)) {
                            bar.place(slot, action);
                        }
                    }
                }
                state.panel_mouse(false, true);
                if let (Some(slot), Some(bar), Some(player)) = (state.mouse_slot.take(), &mut state.actionbar, &mut state.player) {
                    bar.release(slot, player);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (true, Some(old), Some(editor)) = (state.rotating, state.cursor, &mut state.editor) {
                    editor.turn_by(position.x as f32 - old.0);
                }
                state.cursor = (!state.grabbed).then_some((position.x as f32, position.y as f32));
                if let Some(editor) = &mut state.editor {
                    editor.mouse_moved((position.x as f32, position.y as f32));
                }
                if let Some(bar) = &mut state.actionbar {
                    bar.set_cursor(state.cursor);
                }
                let size = (state.config.width, state.config.height);
                if let Some(panels) = &mut state.panels {
                    panels.hover(state.cursor, size);
                }
            }
            // Middle button: holding it looks round with the editor's camera.
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Middle, .. } => {
                let over = state.cursor.zip(state.editor.as_ref().filter(|e| e.open)).is_some_and(|(pos, e)| e.over(pos, state.config.height));
                state.editor_look(button_state == ElementState::Pressed && !over);
            }
            // Right button with the editor open: held, it turns the model that is put down (the pointer stays), else it looks round.
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Right, .. } if state.editor.as_ref().is_some_and(|e| e.open) => {
                let down = button_state == ElementState::Pressed;
                let over = state.cursor.zip(state.editor.as_ref()).is_some_and(|(pos, e)| e.over(pos, state.config.height));
                if !down || !over {
                    if state.editor.as_ref().is_some_and(|e| e.placing()) {
                        state.rotating = down;
                    } else {
                        state.editor_look(down);
                    }
                }
            }
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Right, .. } => {
                let down = button_state == ElementState::Pressed;
                if state.panel_mouse(down, false) {
                } else if !state.grabbed {
                    if down {
                        state.world_click(true);
                    }
                } else if let (Some(p), true) = (&mut state.player, state.grabbed) {
                    p.set_block(down);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                // The pointer left the window: nothing to draw.
                state.cursor = None;
                if let Some(bar) = &mut state.actionbar {
                    bar.set_cursor(None);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                // Over an open window the wheel scrolls it; otherwise it sets the speed of flying.
                let size = (state.config.width, state.config.height);
                let shift = state.input.fast;
                if let (Some(editor), Some(pos)) = (state.editor.as_mut().filter(|e| e.open), state.cursor) {
                    if editor.over(pos, size.1) {
                        editor.wheel(lines);
                        return;
                    }
                    // In the world it sizes (and with Shift, raises) the model that is put down.
                    if editor.scroll_world(lines, shift) {
                        return;
                    }
                }
                let scrolled = match (&mut state.panels, state.cursor) {
                    (Some(panels), Some(pos)) => panels.scroll(pos, size, lines),
                    _ => false,
                };
                if !scrolled {
                    let controller = state.free.as_mut().map_or(&mut state.controller, |f| &mut f.controller);
                    controller.fly_speed = (controller.fly_speed * 1.2f32.powf(lines)).clamp(2.0, 2000.0);
                }
            }
            WindowEvent::Focused(false) => {
                state.input = Input::default();
                state.rotating = false;
                state.ctrl = false;
                if let (Some(bar), Some(p)) = (&mut state.actionbar, &mut state.player) {
                    bar.release_all(p);
                }
                if let Some(p) = &mut state.player {
                    p.set_block(false);
                }
                state.set_grab(false);
            }
            WindowEvent::RedrawRequested => {
                state.frame();
                if state.apply_after.is_some_and(|t| state.start.elapsed().as_secs_f32() > t) {
                    state.apply_after = None;
                    state.apply = true;
                }
                if std::mem::take(&mut state.apply) {
                    state.apply_content();
                }
                if state.quit {
                    return event_loop.exit();
                }
                // The editor saved what it changed and asked for the game to begin again with it, standing where it stood.
                if state.restart {
                    state.save_game();
                    // The same character goes on (not the list again).
                    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--new").collect();
                    if state.logout {
                        // Back to the list: the character is not named, nor is its look.
                        let at = args.iter().position(|a| a == "--character");
                        if let Some(at) = at {
                            args.drain(at..(at + 2).min(args.len()));
                        }
                    } else if !args.iter().any(|a| a == "--character" || a == "--save" || a == "--no-save") {
                        args.extend(["--character".to_string(), state.name.clone()]);
                    }
                    match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(args).spawn()) {
                        Ok(_) => event_loop.exit(),
                        Err(e) => {
                            log::warn!("cannot start the game again: {e}");
                            state.restart = false;
                        }
                    }
                }
            }
            _ => {}
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
