//! The windows that come before the game: the list of characters and the introduction of a race.

use super::*;

/// The window's surface and the GPU that draws to it, in gamma space like the original (a surface that is not sRGB is preferred).
pub(super) fn open_gpu(window: &Arc<Window>) -> Result<(wgpu::Surface<'static>, Gpu, wgpu::SurfaceConfiguration)> {
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
pub(super) struct GlueWindow {
    pub(super) window: Arc<Window>,
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
    pub(super) fn new(window: Arc<Window>, mount: Arc<Mount>, screen: Option<&str>, sound: bool) -> Result<GlueWindow> {
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

    pub(super) fn event(&mut self, event: &WindowEvent) -> Option<Outcome> {
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

    pub(super) fn frame(&mut self) {
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
pub(super) struct CinemaWindow {
    pub(super) window: Arc<Window>,
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
    pub(super) fn new(window: Arc<Window>, mount: Arc<Mount>, race: crate::character::Race, args: &Args) -> Result<CinemaWindow> {
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
    pub(super) fn event(&mut self, event: &WindowEvent) -> bool {
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

    pub(super) fn frame(&mut self) -> bool {
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
pub(super) fn present_loading(surface: &wgpu::Surface, gpu: &Gpu, renderer: &mut Renderer, loading: &Loading, size: (u32, u32)) {
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

/// The weather at the start: fixed by `--weather`, none with `--no-weather`, otherwise it comes and goes by the zone's climate.
pub(super) fn weather_at_start(args: &Args) -> crate::weather::Weather {
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
