//! Opening the game: the window's GPU, the assets, the saved character.

use super::*;

impl State {
    pub(super) fn new(window: Arc<Window>, args: &Args, mount: Arc<Mount>) -> Result<State> {
        let (surface, gpu, config) = open_gpu(&window)?;
        let format = config.format;
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
        // A new character starts in the homeland of its race, unless the command line asked for a place; one that was saved comes back where it was (in a dungeon too).
        let homeland = loaded.is_none() && args.map == "Azeroth" && args.pos == crate::START_POS;
        let (start_map, start_pos, start_yaw) = if homeland { args.look.race.start() } else { (args.map.as_str(), args.pos, args.yaw_deg) };
        let map = loaded.as_ref().and_then(|s| s.map.clone()).unwrap_or_else(|| start_map.to_string());

        let view_distance = args.radius as f32 * wowfmt::adt::TILE_SIZE * 0.95;
        let mut renderer = Renderer::new(&gpu, format, Atmosphere::daytime(view_distance));
        // The loading screen is up while the rest is made ready (and after, until the terrain around the spawn has streamed in).
        let mut loading = Loading::load(&mount, &map, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no loading screen: {e:#}")).ok();
        let mut show = |renderer: &mut Renderer, reached: &[(Stage, f32)]| {
            if let Some(loading) = &mut loading {
                reached.iter().for_each(|&(stage, value)| loading.report(stage, value));
                present_loading(&surface, &gpu, renderer, loading, (config.width, config.height));
            }
        };
        show(&mut renderer, &[]);
        let sky = Sky::load(&mount, &map, view_distance).map_err(|e| log::warn!("no light data: {e:#}")).ok();
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
        let world = World::open(mount, &map, args.radius)?;
        renderer.set_water_textures(&gpu, &world.water_textures());
        show(&mut renderer, &[]);
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
        let mut pickups = Pickups::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &pickup_props, &map).map_err(|e| log::warn!("no pickups: {e:#}")).ok();
        let mut effects = crate::effects::Effects::new(player_mount.clone());
        let lifts = crate::lifts::Lifts::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &mut effects, &map, loaded.as_ref().map_or(0.0, |s| s.lifts_clock)).map_err(|e| log::warn!("no lifts: {e:#}")).ok();
        let mut bodies = Bodies::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &pickup_props, &map).map_err(|e| log::warn!("no physics props: {e:#}")).ok();
        let mut weather = weather_at_start(&args);
        if let Some(saved) = &loaded {
            weather.restore(&saved.weather);
            if let Some(pickups) = &mut pickups {
                pickups.restore(&saved.gone);
            }
            if let Some(bodies) = &mut bodies {
                bodies.restore(&saved.moved);
            }
        }
        let lock_art = crate::lockpick::LockArt::load(&player_mount, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no lockpicking pictures: {e:#}")).ok();
        // The pointer of the original replaces the system's, which is shown only if its pictures are missing.
        let cursors = Cursors::load(&player_mount, &mut renderer.ui, &gpu).map_err(|e| log::warn!("no mouse pointer: {e:#}")).ok();
        window.set_cursor_visible(cursors.is_none());
        show(&mut renderer, &[(Stage::Objects, 2.0 / 3.0), (Stage::Interface, 0.7)]);
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

        let (x, y, z) = start_pos;
        let spawn = wow_to_render([x, y, z.unwrap_or(0.0)]);
        // Where the saved character stood (it drops onto the ground there), or the start.
        let (start, yaw, pitch) = match &loaded {
            Some(saved) => (wow_to_render([saved.position.0, saved.position.1, 0.0]), saved.heading.to_radians(), saved.pitch.to_radians()),
            None => (spawn, start_yaw.to_radians(), args.pitch_deg.to_radians()),
        };
        let mut camera = Camera::new(start, yaw, pitch);
        // The wolves of `content/spawns.ron`.
        let mut wolves = Creatures::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &spawn_list(&map)).map_err(|e| log::warn!("no creatures: {e:#}")).ok();
        if let Some(wolves) = &mut wolves {
            wolves.set_routes(spawn_routes(&map));
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
            lifts,
            bodies,
            pending_loot: None,
            pickup_props,
            pickups_stale: false,
            beds: bed_spots(&content::get().props, &map),
            map,
            home: start_map.to_string(),
            in_portal: None,
            screens: Default::default(),
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
            ragdolls: true,
            apply: false,
            apply_after: args.apply_after,
            mount: player_mount.clone(),
            levelfx: None,
            stealth_sound: crate::visuals::state_sound(&player_mount, crate::visuals::STEALTH),
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
            fixed_time: args.time.is_some() || loaded.as_ref().is_some_and(|s| s.time_fixed),
            weather,
            indoors: false,
            zone: None,
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
            creature_time: Duration::ZERO,
            area_now: None,
            room_now: None,
            place_look: 0.0,
        })
    }
}
