//! `--headless out.png`: stream the world in, render a single frame offscreen, save it.

use crate::collision::Collider;
use crate::camera::{Camera, Controller, Input, Mode, EYE_HEIGHT};
use crate::coords::render_to_wow;
use crate::coords::wow_to_render;
use crate::models::ModelCache;
use crate::player::{Motion, Player, View};
use crate::renderer::{Atmosphere, Gpu, Renderer};
use crate::sky::Sky;
use crate::world::World;
use crate::Args;
use anyhow::{Context, Result};
use mpq::Mount;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn run(args: &Args, mount: Arc<Mount>, out: &Path) -> Result<()> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .context("no suitable GPU adapter")?;
    log::info!("adapter: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))?;
    let gpu = Gpu { device, queue };

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let view_distance = args.radius as f32 * wowfmt::adt::TILE_SIZE * 0.95;
    let mut renderer = Renderer::new(&gpu, format, Atmosphere::daytime(view_distance));
    // The saved video options, as the game takes them into use.
    let video = crate::video::VideoSettings::load();
    (renderer.gamma, renderer.prop_reach, renderer.shadows) = (video.gamma, video.prop_reach(), video.shadows);
    crate::uixml::set_ui_scale(video.interface_scale());
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
    let mut world = World::open(mount, &args.map, args.radius)?;
    renderer.set_water_textures(&gpu, &world.water_textures());

    let (x, y, z) = args.pos;
    let start = wow_to_render([x, y, z.unwrap_or(0.0)]);
    let mut camera = Camera::new(start, args.yaw_deg.to_radians(), args.pitch_deg.to_radians());
    camera.aspect = args.size.0 as f32 / args.size.1 as f32;
    camera.far = view_distance * 1.2;
    if let Some(fov) = args.fov {
        camera.fov_y = fov;
    }

    // Stream everything in.
    let t0 = Instant::now();
    loop {
        world.update(camera.pos.x, camera.pos.z, &gpu, &mut renderer);
        if world.pending_count() == 0 {
            break;
        }
        anyhow::ensure!(t0.elapsed() < Duration::from_secs(180), "timed out loading tiles");
        std::thread::sleep(Duration::from_millis(5));
    }
    log::info!("{} tiles loaded in {:.1}s", world.loaded_count(), t0.elapsed().as_secs_f32());

    if z.is_none() {
        let ground = world.height_at(camera.pos.x, camera.pos.z).context("no terrain at the requested position")?;
        camera.pos.y = ground + args.alt.max(EYE_HEIGHT.min(args.alt));
    }
    if let Some(clock) = args.lifts {
        let mut effects = crate::effects::Effects::new(player_mount.clone());
        let mut lifts = crate::lifts::Lifts::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &mut effects, &args.map, clock)?;
        lifts.advance(0.0, glam::Vec3::splat(1e9), false, |_, _| true, &mut renderer, &mut effects);
        for _ in 0..30 {
            effects.update(1.0 / 30.0, &camera, &mut renderer, &gpu);
        }
    }
    // The weather, if one was asked for: a few seconds of it, so that the rain is in the air.
    let mut weather = crate::weather::Weather::new(1);
    weather.auto = false;
    if let Some((kind, strength)) = args.weather {
        weather.set(kind, strength, true);
        let (sy, cy) = camera.yaw.sin_cos();
        let motion = crate::weather::Motion { eye: camera.pos, velocity: glam::Vec3::ZERO, forward: glam::Vec3::new(sy, 0.0, -cy), indoors: world.indoors(camera.pos) };
        let began = Instant::now();
        for _ in 0..240 {
            weather.update(1.0 / 30.0, &motion, &world, None);
        }
        log::info!("weather: {} particles, {} splashes, {} puffs of mist; {:.2} ms a frame", weather.particles.len(), weather.splashes.len(), weather.puffs.len(), began.elapsed().as_secs_f32() * 1000.0 / 240.0);
    }
    if let Some(a) = sky.as_ref().and_then(|s| s.atmosphere(camera.pos, args.time.unwrap_or(12.0), weather.storm())) {
        renderer.atmosphere = a;
    }
    if let Some(a) = world.area_at(camera.pos) {
        log::info!("standing in: {}", world.areas.display(a));
        if let Ok(bank) = crate::audio::SoundBank::load(&player_mount) {
            log::info!("music of the area: {:?}", bank.music_for(a));
        }
    }
    log::info!("indoors: {}; room music {:?}, ambience {:?}", world.indoors(camera.pos), world.room_at(camera.pos).map(|r| r.music), world.room_at(camera.pos).map(|r| r.ambience));
    let waters = world.nearby_liquids(camera.pos, 9.0);
    let sounds = world.nearby_sounds(camera.pos, 130.0);
    for p in world.sample_building_liquids() {
        log::info!("building liquid at wow ({:.0}, {:.0}) height {:.1}", -p.z, -p.x, p.y);
    }
    log::info!("{} liquid cells of buildings in the loaded tiles", world.building_liquid_cells());
    log::info!("liquid within 9 yards: {:?}; {} sounds of models within 130; liquid level here: {:?}", waters.iter().map(|k| k.iter().filter(|s| s.is_some()).count()).collect::<Vec<_>>(), sounds.len(), world.liquid_level(camera.pos.x, camera.pos.z));

    let z_given = z.is_some(); // with an explicit height, the floor map only shows floors up to it
    if args.floor_map {
        // 1 yard per character, north at the top. ' ' = nothing, '.' = terrain level,
        // digit = floor that many yards above the terrain, '+' = 10 or more, '-' = below it.
        let (cx, cz) = (camera.pos.x, camera.pos.z);
        println!("floor map centred on ({:.0}, {:.0}); north up, 1 char = 1 yard, 80x50", render_to_wow(camera.pos)[0], render_to_wow(camera.pos)[1]);
        for row in -25..25 {
            let line: String = (-40..40)
                .map(|col| {
                    let (x, z) = (cx + col as f32 * args.floor_scale, cz + row as f32 * args.floor_scale);
                    let terrain = world.height_at(x, z);
                    match (world.floor(x, z, if z_given { camera.pos.y + 1.0 } else { 10_000.0 }), terrain) {
                        (None, _) => ' ',
                        (Some(_), None) => '#',
                        (Some(f), Some(t)) => match f - t {
                            d if d < -0.3 => '-',
                            d if d < 0.3 => '.',
                            d if d < 9.5 => char::from_digit((d + 0.5) as u32, 10).unwrap_or('+'),
                            _ => '+',
                        },
                    }
                })
                .collect();
            println!("{line}");
        }
    }

    if args.find_holes {
        // A hole: nothing walkable within [top-3.5, top] at this spot, yet walkable surface at that height on both sides.
        let top = camera.pos.y;
        let has = |x: f32, z: f32| world.floor(x, z, top).is_some_and(|f| f > top - 3.5);
        let mut holes = Vec::new();
        for iz in -75..=75 {
            for ix in -75..=75 {
                let (x, z) = (camera.pos.x + ix as f32 * 0.2, camera.pos.z + iz as f32 * 0.2);
                if has(x, z) {
                    continue;
                }
                let d = 0.5;
                if (has(x - d, z) && has(x + d, z)) || (has(x, z - d) && has(x, z + d)) {
                    holes.push((x, z));
                }
            }
        }
        log::info!("holes: {} grid cells (0.2 yd) have no floor but floor on opposite sides", holes.len());
        // Cluster into 1 yd cells for a readable list.
        let mut clusters = std::collections::BTreeMap::<(i32, i32), usize>::new();
        for (x, z) in &holes {
            *clusters.entry((x.floor() as i32, z.floor() as i32)).or_default() += 1;
        }
        for ((cx, cz), n) in clusters.iter().take(25) {
            let [wx, wy, _] = render_to_wow(glam::Vec3::new(*cx as f32 + 0.5, 0.0, *cz as f32 + 0.5));
            log::info!("  {n:>3} cells near wow ({wx:.1}, {wy:.1})");
        }
    }

    if args.stress_walk {
        // Random steering from random starts on the floor near --pos; report drops after a climb.
        let mut rng = 0x2545_F491u32;
        let mut next = move || {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (rng >> 8) as f32 / (1u32 << 24) as f32
        };
        let centre = camera.pos;
        let (mut runs, mut falls) = (0, 0);
        let mut by_height = std::collections::BTreeMap::<i32, usize>::new();
        for _ in 0..600 {
            let (x, zz) = (centre.x + (next() - 0.5) * 6.0, centre.z + (next() - 0.5) * 6.0);
            let Some(ground) = world.floor(x, zz, centre.y) else { continue };
            let mut cam = Camera::new(glam::Vec3::new(x, ground + EYE_HEIGHT, zz), (80.0 + next() * 80.0).to_radians(), 0.0);
            let mut c = Controller::new(Mode::Walk);
            let mut input = Input { forward: true, ..Default::default() };
            runs += 1;
            let mut best = ground;
            let mut prev = cam.pos;
            let mut fast_frames = 0;
            let mut trace: Vec<String> = Vec::new();
            let mut history: Vec<f32> = Vec::new();
            for frame in 0..(4 * 60) {
                if frame % 20 == 0 {
                    cam.yaw += (next() - 0.5) * 50f32.to_radians();
                }
                prev = cam.pos;
                c.update(&mut cam, &mut input, &world, 1.0 / 60.0);
                let feet = cam.pos.y - EYE_HEIGHT;
                history.push(feet);
                {
                    let [tx, ty, tz] = render_to_wow(glam::Vec3::new(cam.pos.x, feet, cam.pos.z));
                    trace.push(format!(
                        "f{frame:03} wow ({tx:.2},{ty:.2}) feet {tz:.3} floor@step {:?} yaw {:.0}",
                        world.floor(cam.pos.x, cam.pos.z, feet + 0.8).map(|v| (v * 1000.0).round() / 1000.0),
                        cam.yaw.to_degrees().rem_euclid(360.0)
                    ));
                }
                best = best.max(feet);
                // A fall is sustained downward speed beyond what descending stairs reaches (~7 yd/s at 45 degrees).
                let speed = (prev.y - cam.pos.y) * 60.0;
                fast_frames = if speed > 9.0 { fast_frames + 1 } else { 0 };
                if best - ground > 0.6 && fast_frames >= 3 {
                    falls += 1;
                    if falls == 3 {
                        for line in trace.iter().rev().take(14).rev() {
                            log::info!("  {line}");
                        }
                    }
                    let pf = prev.y - EYE_HEIGHT;
                    let started_from = history[history.len().saturating_sub(25)];
                    let _ = pf;
                    *by_height.entry((started_from * 2.0).round() as i32).or_default() += 1;
                    let [wx, wy, wz] = render_to_wow(glam::Vec3::new(prev.x, pf, prev.z));
                    log::debug!(
                        "fall at t={:.2}s: feet were ({wx:.2}, {wy:.2}, {wz:.2}) yaw {:.0}; floor within step {:?}, floor within 3 yd {:?}, floor below {:?}",
                        frame as f32 / 60.0,
                        cam.yaw.to_degrees().rem_euclid(360.0),
                        world.floor(prev.x, prev.z, pf + 0.8),
                        world.floor(prev.x, prev.z, pf + 3.0),
                        world.floor(prev.x, prev.z, pf - 0.01)
                    );
                    break;
                }
            }
            let _ = prev;
        }
        log::info!("stress: {falls} falls in {runs} runs");
        for (h, n) in &by_height {
            log::info!("  fall started near feet height {:.1}: {n}", *h as f32 / 2.0);
        }
    }

    // Whether anything of the content stands in or under a building, a tree or a wall: objects with something at head height
    // within two yards of them.
    if args.check_spawns {
        let content = crate::content::get();
        let places = content.spawns.iter().map(|s| (s.creature.as_str(), s.at)).chain(content.chests.iter().map(|c| (c.id.as_str(), c.at)));
        for (name, at) in places {
            let p = crate::coords::wow_to_render([at.0, at.1, 0.0]);
            let Some(ground) = world.height_at(p.x, p.z) else { log::warn!("{name} at {at:?}: no ground loaded"); continue };
            let blocked = world.triangles_near(p.x, p.z, 2.0).iter().any(|t| t.iter().all(|v| v.y > ground + 0.5 && v.y < ground + 2.5));
            if blocked {
                log::warn!("{name} at {at:?}: something stands there");
            }
        }
        log::info!("checked the places of the content");
    }

    if args.dump_tris {
        let mut tris = world.triangles_near(camera.pos.x, camera.pos.z, 3.0);
        tris.sort_by(|a, b| (a[0].y + a[1].y + a[2].y).total_cmp(&(b[0].y + b[1].y + b[2].y)));
        for t in tris.iter().take(60) {
            let w: Vec<String> = t.iter().map(|v| { let [x, y, z] = render_to_wow(*v); format!("({x:.2},{y:.2},{z:.2})") }).collect();
            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            let slope = n.y.abs().clamp(0.0, 1.0).acos().to_degrees();
            log::info!("tri {} {} {}  slope {slope:.0} deg", w[0], w[1], w[2]);
        }
    }

    if args.find_ramps {
        let mut clusters: std::collections::BTreeMap<(i32, i32, i32), usize> = std::collections::BTreeMap::new();
        for c in world.ramp_centroids(camera.pos.x, camera.pos.z, 40.0, 35.0, 50.0) {
            *clusters.entry(((c.x / 2.0).floor() as i32, (c.z / 2.0).floor() as i32, (c.y / 1.0).floor() as i32)).or_default() += 1;
        }
        let mut top: Vec<_> = clusters.into_iter().collect();
        top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        log::info!("ramp-like surfaces near the camera (2x2 yd cells, by height):");
        for ((cx, cz, cy), n) in top.iter().take(15) {
            let [wx, wy, wz] = render_to_wow(glam::Vec3::new(*cx as f32 * 2.0 + 1.0, *cy as f32 + 0.5, *cz as f32 * 2.0 + 1.0));
            log::info!("  {n:>3} triangles around ({wx:.0}, {wy:.0}) at height {wz:.1}");
        }
    }

    if args.floor_profile {
        let heading = args.yaw_deg.to_radians();
        let dir = glam::Vec3::new(heading.sin(), 0.0, -heading.cos());
        let mut line = String::new();
        for i in 0..=80 {
            let p = camera.pos + dir * (i as f32 * 0.25);
            let max_y = if z.is_some() { camera.pos.y + 0.3 } else { 10_000.0 };
            match world.floor(p.x, p.z, max_y) {
                Some(f) => line.push_str(&format!("{f:.2} ")),
                None => line.push_str("-- "),
            }
        }
        log::info!("floor every 0.25 yd along the heading (max height {:.1}):
{line}", camera.pos.y + 0.3);
    }

    if args.probe_walk {
        // Experiment: can the player reach raised walkable surfaces by walking alone?
        let centre = camera.pos;
        let (mut runs, mut raised, mut roofed, mut climbed) = (0, Vec::new(), Vec::new(), Vec::<(f32, [f32; 3])>::new());
        for dz in (-40..=40).step_by(2) {
            for dx in (-40..=40).step_by(2) {
                for heading in 0..8 {
                    let (x, z) = (centre.x + dx as f32, centre.z + dz as f32);
                    // With an explicit height in --pos, start on the highest floor at or below it.
                    let start_ground = if z_given { world.floor(x, z, centre.y) } else { world.height_at(x, z) };
                    let Some(ground) = start_ground else { continue };
                    let mut cam = Camera::new(glam::Vec3::new(x, ground + EYE_HEIGHT, z), heading as f32 * std::f32::consts::FRAC_PI_4, 0.0);
                    let mut c = Controller::new(Mode::Walk);
                    let mut input = Input { forward: true, ..Default::default() };
                    for _ in 0..(3 * 60) {
                        c.update(&mut cam, &mut input, &world, 1.0 / 60.0);
                    }
                    runs += 1;
                    if z_given && cam.pos.y - EYE_HEIGHT - ground > 2.0 {
                        climbed.push((cam.pos.y - EYE_HEIGHT - ground, render_to_wow(cam.pos - glam::Vec3::Y * EYE_HEIGHT)));
                    }
                    let feet = cam.pos.y - EYE_HEIGHT;
                    let terrain = world.height_at(cam.pos.x, cam.pos.z).unwrap_or(feet);
                    // A walkable surface 2.5..15 yards overhead means we are indoors (or under a bridge).
                    if let Some(above) = world.floor(cam.pos.x, cam.pos.z, feet + 15.0) {
                        if above - feet > 2.5 && (feet - terrain).abs() < 0.6 {
                            roofed.push(render_to_wow(cam.pos - glam::Vec3::Y * EYE_HEIGHT));
                        }
                    }
                    if feet - terrain > 1.2 && feet - terrain < 30.0 {
                        raised.push((feet - terrain, render_to_wow(cam.pos - glam::Vec3::Y * EYE_HEIGHT)));
                    }
                }
            }
        }
        if z_given {
            climbed.sort_by(|a, b| b.0.total_cmp(&a.0));
            log::info!("probe: {} of {runs} walks from the interior floor climbed more than 2 yd", climbed.len());
            // distinct landing spots, 3 yd cells
            let mut cells = std::collections::BTreeMap::<(i32, i32), (f32, [f32; 3])>::new();
            for c in &climbed {
                cells.entry(((c.1[0] / 3.0) as i32, (c.1[1] / 3.0) as i32)).or_insert(*c);
            }
            for (h, [x, y, zz]) in cells.values().take(10) {
                log::info!("  climbed +{h:.1} yd, ended at ({x:.1}, {y:.1}, {zz:.1})");
            }
        }
        log::info!("probe: {} walks ended under a roof at ground level", roofed.len());
        for [x, y, z] in roofed.iter().take(6) {
            log::info!("  indoors at ({x:.1}, {y:.1}, {z:.1})");
        }
        raised.sort_by(|a, b| b.0.total_cmp(&a.0));
        log::info!("probe: {runs} walks, {} ended standing on a floor more than 1.2 yd above the terrain", raised.len());
        for (h, [x, y, z]) in raised.iter().take(8) {
            log::info!("  +{h:.1} yd at ({x:.1}, {y:.1}, {z:.1})");
        }
    }

    if args.walk > 0.0 {
        // Run the real controller against the real collision data, 60 steps per second.
        let mut controller = Controller::new(Mode::Walk);
        let mut input = Input { forward: true, ..Default::default() };
        let start = camera.pos;
        let mut swam = 0.0;
        for _ in 0..(args.walk * 60.0) as usize {
            let feet = camera.pos - glam::Vec3::Y * EYE_HEIGHT;
            input.water = world.liquid_at(feet.x, feet.z, feet.y);
            controller.update(&mut camera, &mut input, &world, 1.0 / 60.0);
            swam += controller.swimming() as u32 as f32 / 60.0;
        }
        log::info!("swam {swam:.1} s of it, water level at the end {:?}", input.water);
        let [sx, sy, sz] = render_to_wow(start - glam::Vec3::Y * EYE_HEIGHT);
        let [ex, ey, ez] = render_to_wow(camera.pos - glam::Vec3::Y * EYE_HEIGHT);
        let dist = ((ex - sx).powi(2) + (ey - sy).powi(2)).sqrt();
        log::info!(
            "walked {:.1}s: ({sx:.1}, {sy:.1}, {sz:.1}) -> ({ex:.1}, {ey:.1}, {ez:.1}), {dist:.1} yards on the ground plane (free run would be {:.1})",
            args.walk,
            args.walk * 7.0
        );
        let feet = camera.pos - glam::Vec3::Y * EYE_HEIGHT;
        log::info!(
            "under the feet: terrain {:?}, highest floor {:?}, feet at {:.2}",
            world.height_at(feet.x, feet.z).map(|h| (h * 100.0).round() / 100.0),
            world.floor(feet.x, feet.z, feet.y + 1.0).map(|h| (h * 100.0).round() / 100.0),
            feet.y
        );
        log::info!("{} collision triangles loaded", world.collision_triangles());
        if let Some(a) = world.area_at(camera.pos) {
            log::info!("now in: {}", world.areas.display(a));
        }
    }

    let (w, h) = args.size;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut render_camera = Camera { pos: camera.pos, ..camera };
    if let Some(v) = &args.view {
        let mut player = Player::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &args.look)?;
        player.view = if v == "third" { View::Third } else { View::First };
        player.body_turn = args.pose_turn.to_radians();
        player.set_sneaking(args.sneak);
        if let Some((display, percent)) = args.mount {
            player.ride(display, percent, &mut renderer, &gpu)?;
        }
        let motion = match args.pose.as_str() {
            "walk" => Motion { speed: 2.5, forward_speed: 2.5, ..Default::default() },
            "run" => Motion { speed: 7.0, forward_speed: 7.0, ..Default::default() },
            "back" => Motion { speed: 2.5, forward_speed: -2.5, ..Default::default() },
            "strafe" => Motion { speed: 7.0, side_speed: 7.0, ..Default::default() },
            "jump" => Motion { airborne: true, vertical_speed: 3.0, ..Default::default() },
            "fall" => Motion { airborne: true, vertical_speed: -15.0, ..Default::default() },
            "swim" => Motion { speed: 4.7, forward_speed: 4.7, swimming: true, ..Default::default() },
            "swim-idle" => Motion { swimming: true, ..Default::default() },
            "swim-back" => Motion { speed: 2.5, forward_speed: -2.5, swimming: true, ..Default::default() },
            "swim-left" => Motion { speed: 4.7, side_speed: -4.7, swimming: true, ..Default::default() },
            _ => Motion::default(),
        };
        // The preview turns the view at this rate (radians per second) for the "turn-left" / "turn-right" poses.
        let turn = match args.pose.as_str() {
            "turn-left" => -2.0,
            "turn-right" => 2.0,
            _ => 0.0,
        };
        // The character sheet and the backpack, for the panel previews; worn items dress the character first.
        let mut panels = if !args.panel.is_empty() || !args.equip.is_empty() || !args.wear.is_empty() || args.npc || args.level.is_some() || args.target || args.compass || !args.cast.is_empty() { Some(crate::panels::Panels::new(player_mount.clone(), &mut renderer, &gpu)?) } else { None };
        if let Some(panels) = &mut panels {
            for &slot in &args.equip {
                let _ = panels.inv.equip_from_bag(slot, 60);
            }
            for id in &args.wear {
                match crate::items::all().iter().position(|i| i.id == *id) {
                    Some(item) => {
                        panels.inv.bag = vec![None; crate::items::BACKPACK_SIZE];
                        panels.inv.add(crate::items::Stack { item, count: 1 });
                        if let Err(e) = panels.inv.equip_from_bag(0, 99) {
                            log::warn!("{id} is not put on: {e:?}");
                        }
                    }
                    None => log::warn!("no item {id}"),
                }
            }
            if let Some(level) = args.level {
                panels.progress = crate::levels::Progress { level, xp: args.xp };
            }
            // A quest taken ("taken": the grey mark over its giver) or ready to turn in ("ready": the yellow one).
            let wolves_quest = crate::content::quest_index("q33");
            if args.panel.iter().any(|p| p == "taken" || p == "ready") {
                panels.quests.accept(wolves_quest);
            }
            if args.panel.iter().any(|p| p == "ready") {
                panels.inv.add(crate::items::Stack { item: crate::items::find("tough_wolf_meat"), count: 8 });
            }
            panels.refresh();
            player.set_equipment(&mut renderer, &gpu, &panels.inv.equipment, &panels.stats);
        }
        let feet = camera.pos - glam::Vec3::Y * EYE_HEIGHT;
        let (sy, cy) = camera.yaw.sin_cos();
        let yaw = camera.yaw;
        let mut look = camera;
        let mut text = crate::combattext::CombatText::load(&player_mount, &mut renderer.ui, &gpu)?;
        let (wolf, guard) = (crate::content::creature_index("wolf"), crate::content::creature_index("deputy_willem"));
        let (ahead, aside) = (glam::Vec3::new(sy, 0.0, -cy), glam::Vec3::new(cy, 0.0, sy));
        let mut spawns = Vec::new();
        if args.wolf {
            // A second wolf far away stays idle: two wolves must not share a pose.
            spawns.extend([(wolf, feet + ahead * 4.0 + aside * 1.5, yaw + std::f32::consts::PI, 5.0), (wolf, feet + ahead * 30.0, yaw, 5.0)]);
        }
        if args.spawns {
            spawns.extend(crate::content::get().spawns.iter().filter(|s| s.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(&args.map))).map(|s| (crate::content::creature_index(&s.creature), crate::coords::wow_to_render([s.at.0, s.at.1, s.height.unwrap_or(0.0)]), s.facing.to_radians(), s.wander)));
        }
        // Any creatures of the content, standing in a row ahead (`--creature kobold_miner,garrick_thorne`).
        for (i, id) in args.creatures.iter().enumerate() {
            spawns.push((crate::content::creature_index(id), feet + ahead * 6.0 + aside * (i as f32 * 2.2 - args.creatures.len() as f32 + 1.0), yaw + std::f32::consts::PI, 5.0));
        }
        if args.npc {
            spawns.push((guard, feet + ahead * 5.0 - aside * 2.0, yaw + std::f32::consts::PI, 5.0));
        }
        let mut wolves = if spawns.is_empty() { None } else { Some(crate::creature::Creatures::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu, &spawns)?) };
        if let Some(wolves) = &mut wolves {
            wolves.set_budget(usize::MAX);
        }
        // The chests of the world, shut or open.
        if let Some(state) = &args.chests {
            let mut chests = crate::chest::Chests::load(&player_mount, &ModelCache::default(), &mut renderer, &gpu)?;
            if state == "open" {
                (0..2).for_each(|i| drop(chests.open(i, &[])));
            }
            for _ in 0..120 {
                chests.update(1.0 / 60.0, |x, z| world.height_at(x, z), &mut renderer, &gpu);
            }
        }
        let mut effects = crate::effects::Effects::new(player_mount.clone());
        let mut foes = crate::foes::Foes::new(crate::visuals::load_classic(&player_mount).unwrap_or_default());
        if let Some(wolves) = &mut wolves {
            wolves.set_spell_anims(foes.anims());
        }
        let mut fires = crate::fires::Fires::default();
        let mut ripples = crate::ripples::Ripples::default();
        let mut ripple_draw = crate::ripples::RippleDraw::new(&player_mount, &ModelCache::default(), &mut renderer, &gpu);
        let mut caster = crate::casting::Spellcaster::new(crate::visuals::load(&player_mount)?);
        // An effect of the game's models (`--effect Spells\Fire_ImpactDD_Low_Chest.m2,300`) or the level up (`--levelfx 400`), caught that
        // many milliseconds after it began: six yards ahead, or round the character.
        let preview = |effects: &mut crate::effects::Effects, renderer: &mut crate::renderer::Renderer, path: &str, ms: f32, at: glam::Vec3| {
            let id = effects.spawn(renderer, &gpu, path, false, glam::Mat4::from_translation(at));
            for _ in 0..(ms / 16.7) as usize {
                effects.update(1.0 / 60.0, &look, renderer, &gpu);
            }
            log::info!("effect {path}: {:?}, {} particles alive at {at:?}", id, id.map_or(0, |i| effects.particles(i)));
        };
        if let Some((path, ms)) = &args.effect {
            preview(&mut effects, &mut renderer, path, *ms, feet + ahead * 6.0 + glam::Vec3::Y);
        }
        if let Some(ms) = args.levelfx {
            preview(&mut effects, &mut renderer, crate::effects::LEVEL_UP, ms, feet);
        }
        match args.pose.as_str() {
            "attack" => {
                player.attack_press();
                player.attack_release();
            }
            "heavy" => player.heavy_attack(),
            "block" => player.set_block(true),
            "sheath" => player.toggle_sheath(),
            "bow" | "shoot" => player.shoot_press(),
            p if p.starts_with("anim") => player.set_precast(Some(p[4..].parse().unwrap_or(0))),
            _ => {}
        }
        // Spells cast one after the other (`--cast fireball,holy_light`) at the first creature, which is selected.
        let mut queue: Vec<usize> = args.cast.iter().map(|id| crate::content::spell_index(id)).rev().collect();
        if let (false, Some(wolves)) = (queue.is_empty(), &mut wolves) {
            wolves.set_target(0);
        }
        if let (true, Some(wolves)) = (args.kill, &mut wolves) {
            // Whatever stands within reach ahead (the first of `--creature`) dies at once.
            wolves.strike(feet + ahead * 3.0, yaw, 1e4, 1.0, &mut |_, _| Some(0.0));
        }
        for frame in 0..(args.pose_time * 60.0).max(1.0) as usize {
            look.yaw += turn / 60.0;
            // `--pose shoot`: the string is let go after two seconds.
            if args.pose == "shoot" && frame == 120 {
                player.shoot_release();
            }
            render_camera = player.update(1.0 / 60.0, &look, motion, |from, to| world.camera_distance(from, to), &mut renderer, &gpu);
            if let Some(panels) = &mut panels {
                panels.update_cast_bar(1.0 / 60.0);
                let mut ctx = crate::casting::Ctx { player: &mut player, panels, wolves: wolves.as_mut(), text: Some(&mut text), effects: &mut effects, renderer: &mut renderer, gpu: &gpu };
                if let (false, Some(spell)) = (caster.casting(), queue.last().copied()) {
                    queue.pop();
                    ctx.panels.cooldowns.iter_mut().for_each(|c| *c = 0.0);
                    caster.begin(&mut ctx, spell, feet);
                }
                if let Some(shot) = ctx.player.take_shot() {
                    caster.shoot(&mut ctx, shot, feet, yaw);
                }
                caster.update(&mut ctx, 1.0 / 60.0, feet, false);
            }
            if let (Some(kind), Some(level)) = (&args.ripples, world.liquid_at(feet.x, feet.z, feet.y)) {
                let (state, speed) = match kind.as_str() {
                    "walk" => (crate::ripples::State::Moving, 2.5),
                    "run" => (crate::ripples::State::Moving, 7.0),
                    _ => (crate::ripples::State::Idle, 0.0),
                };
                let walker = crate::ripples::Walker { feet: glam::Vec3::new(feet.x, level - 0.6, feet.z), level, height: crate::movement::DEFAULT_HEIGHT, scale: 1.0, speed, heading: (sy, -cy) };
                ripples.update(1.0 / 60.0);
                ripples.make(&walker, state, |at, half| world.liquid_quads(at.x, at.z, level - 0.6, half));
                if let Some(draw) = &mut ripple_draw {
                    draw.draw(&ripples, feet, &mut renderer, &gpu);
                }
            }
            fires.update(&world, render_camera.pos, &mut effects, &mut renderer, &gpu);
            effects.update(1.0 / 60.0, &render_camera, &mut renderer, &gpu);
            if let Some(wolves) = &mut wolves {
                if player.take_swing_hit().is_some() {
                    let damage = player.roll_damage();
                    for (at, ..) in wolves.strike(feet, yaw, damage, 1.0, &mut |_, _| Some(0.0)) {
                        text.damage_dealt(at, damage as u32);
                    }
                }
                text.update(1.0 / 60.0);
                if let Some(panels) = &panels {
                    wolves.set_markers(render_camera.pos, |kind| panels.marker(kind));
                }
                let watch = crate::creature::Watch { feet, yaw, level: 1, health: player.vitals.health.fraction(), mana: 1.0, casting: false, auras: &foes.afflictions.spells(), sneaking: args.sneak.then_some(1.0) };
                let (acts, _) = wolves.update(1.0 / 60.0, &watch, |x, z, y| crate::collision::Collider::floor(&world, x, z, y + 1.0).or_else(|| world.height_at(x, z)), |from, to| world.blocked(from, to), None, &mut renderer, &gpu);
                let mut effect_list: Vec<crate::foes::Effect> = acts.into_iter().flat_map(|act| foes.handle(act, feet, wolves, &mut effects, &mut renderer, &gpu)).collect();
                effect_list.extend(foes.update(1.0 / 60.0, feet, wolves, &mut effects, &mut renderer, &gpu));
                for effect in effect_list {
                    if let crate::foes::Effect::Hurt { amount, physical, .. } = effect {
                        if physical { player.take_hit(amount); } else { player.take_magic(amount); }
                    }
                }
            }
        }
        log::info!("fires: {} burning of {} in the loaded tiles", fires.burning(), world.fires().count());
        if let Some(wolves) = &wolves {
            log::info!("creatures in the world: {} made, {} with a model", wolves.counts().0, wolves.counts().1);
            wolves.draw_plates(&mut renderer.ui, &render_camera, (w, h));
            if let Some(panels) = &mut panels {
                panels.draw_labels(&mut renderer.ui, &gpu, (w, h), &wolves.labels(&render_camera, (w, h)));
            }
            text.draw(&mut renderer.ui, &render_camera, (w, h));
        }
        // The target frame with the first creature selected.
        if let (Some(panels), Some(wolves), true) = (&mut panels, &mut wolves, args.target && args.panel.is_empty()) {
            wolves.set_target(0);
            let view = wolves.target_view(feet);
            panels.set_target(view);
            panels.render_target(&mut renderer, &gpu, wolves, &world);
            panels.draw(&mut renderer.ui, &gpu, (w, h));
        }
        if let (Some(panels), false) = (&mut panels, args.panel.is_empty()) {
            // A character who has learned every spell and has trained a few skills, for the pictures of those windows.
            if args.panel.iter().any(|p| p == "spellbook" || p == "skills") {
                for spell in 0..crate::content::get().spells.len() {
                    panels.learn(spell);
                }
                for (i, skill) in crate::skills::ALL.iter().enumerate() {
                    panels.skills.train(*skill, (i * i * 7) as f32);
                }
            }
            // Two tomes in the pack, read: a basic one is learned, a greater one asks for a rank the character has not got.
            if args.panel.iter().any(|p| p == "tome") {
                for id in ["tome_fireball", "tome_arcane_explosion"] {
                    panels.inv.add(crate::items::Stack { item: crate::items::find(id), count: 1 });
                }
                let first = panels.inv.bag.iter().position(|s| s.is_some_and(|s| crate::items::get(s.item).tome.is_some())).unwrap_or(0);
                panels.read_tome(first);
                panels.read_tome(first + 1);
                panels.toggle_backpack();
            }
            if args.panel.iter().any(|p| p == "character" || p == "skills") {
                panels.toggle_character();
            }
            if args.panel.iter().any(|p| p == "skills") {
                panels.show_skills_tab(true);
            }
            if args.panel.iter().any(|p| p == "spellbook") {
                panels.toggle_spellbook();
            }
            if args.panel.iter().any(|p| p == "map" || p == "map-zoom") {
                panels.found.iter_mut().take(args.discover).for_each(|f| *f = true);
                panels.explore(world.area_at(render_camera.pos));
                panels.toggle_map();
                if args.panel.iter().any(|p| p == "map-zoom") {
                    panels.map_zoom(args.cursor.unwrap_or((640.0, 360.0)), (w, h), 6.0);
                }
            }
            if args.panel.iter().any(|p| p == "menu" || p == "sound" || p == "video" || p == "interface" || p == "dropdown") {
                panels.open_menu();
                if args.panel.iter().any(|p| p == "sound") {
                    panels.menu_click("GameMenuButtonSoundOptions");
                }
                if args.panel.iter().any(|p| p == "video") {
                    panels.menu_click("GameMenuButtonOptions");
                }
                if args.panel.iter().any(|p| p == "dropdown") {
                    panels.menu_click("GameMenuButtonOptions");
                    panels.set_video_modes(vec![(1920, 1080, 165), (1920, 1080, 60), (1280, 720, 60)]);
                    panels.menu_click(if args.panel.iter().any(|p| p == "mode") { "OptionsFrameMultiSampleDropDownButton" } else { "OptionsFrameResolutionDropDownButton" });
                }
                if args.panel.iter().any(|p| p == "interface") {
                    panels.menu_click("GameMenuButtonUIOptions");
                }
            }
            if args.panel.iter().any(|p| p == "bag") {
                panels.toggle_backpack();
            }
            if args.panel.iter().any(|p| p == "loot") {
                let stack = |item, count| crate::items::Stack { item, count };
                panels.open_loot(0, crate::loot::Loot { copper: 7, items: vec![stack(crate::items::find("tough_wolf_meat"), 2), stack(crate::items::find("ruined_pelt"), 1)] });
            }
            if args.panel.iter().any(|p| p == "gossip") {
                panels.open_gossip(0, crate::content::creature("deputy_willem"));
            }
            if args.panel.iter().any(|p| p == "trainer") {
                panels.open_gossip(0, crate::content::creature("instructor_hale"));
            }
            // An elixir, a scroll and food at work, for the icons of the buffs.
            if args.panel.iter().any(|p| p == "buffs") {
                for id in ["i3825", "i954", "i117"] {
                    let item = crate::items::find(id);
                    panels.inv.add(crate::items::Stack { item, count: 1 });
                    let slot = panels.inv.bag.iter().position(|s| s.is_some_and(|s| s.item == item)).unwrap_or(0);
                    let _ = panels.consumed.use_item(&mut panels.inv, slot, 60, true);
                }
                panels.refresh();
            }
            for (name, page) in [("merchant", 0), ("merchant2", 1), ("merchant3", 2)] {
                if args.panel.iter().any(|p| p == name) {
                    panels.open_merchant(0, crate::content::creature("trader_aldric"));
                    panels.merchant_page_to(page);
                }
            }
            // The quest window at each of its screens, and the tracker with a quest half done.
            let (willem, wolves_quest) = (crate::content::creature("c196"), crate::content::quest_index("q33"));
            for (name, screen) in [("quest", crate::panels::QuestScreen::Detail), ("quest-progress", crate::panels::QuestScreen::Progress), ("quest-reward", crate::panels::QuestScreen::Reward)] {
                if args.panel.iter().any(|p| p == name) {
                    panels.quests.accept(wolves_quest);
                    panels.open_quest(0, willem, wolves_quest, screen);
                }
            }
            if args.panel.iter().any(|p| p == "log") {
                panels.quests.accept(wolves_quest);
                panels.quests.accept(crate::content::quest_index("q5261"));
                panels.on_kills(&[crate::content::creature_index("c6")]);
                panels.toggle_quest_log();
            }
            if args.panel.iter().any(|p| p == "abandon") {
                panels.quests.accept(wolves_quest);
                panels.toggle_quest_log();
                panels.layout(&mut renderer.ui, &gpu, (w, h));
                // The log's "Abandon Quest" button asks for confirmation.
                panels.mouse_down((75.0, 513.0), (w, h));
                panels.mouse_up((75.0, 513.0), (w, h), true);
            }
            if args.panel.iter().any(|p| p == "sleep") {
                panels.ask_sleep();
            }
            if args.panel.iter().any(|p| p == "levelup" || p == "levelhint") {
                panels.gain_xp(1000);
            }
            if args.panel.iter().any(|p| p == "levelup") {
                panels.toggle_level();
            }
            if args.panel.iter().any(|p| p == "lockpick") {
                // A lock half turned with the pick off the sweet spot: the pick strains.
                let mut lock = crate::lockpick::Lockpick::new(crate::lockpick::Level::Adept, 20.0);
                lock.rotate(60.0);
                lock.update(0.4, true);
                let art = crate::lockpick::LockArt::load(&player_mount, &mut renderer.ui, &gpu)?;
                panels.draw_lockpick(&mut renderer.ui, &gpu, (w, h), &lock, Some(&art), 0.01);
            }
            if args.panel.iter().any(|p| p == "tracker") {
                panels.quests.accept(wolves_quest);
                panels.quests.accept(crate::content::quest_index("q7"));
                panels.on_kills(&[crate::content::creature_index("c6")]);
            }
            if args.panel.iter().any(|p| p == "error") {
                panels.error("Inventory is full.");
            }
            panels.layout(&mut renderer.ui, &gpu, (w, h));
            for &pos in &args.clicks {
                // A negative x is a right click at that distance from the left.
                let (pos, left) = ((pos.0.abs(), pos.1), pos.0 >= 0.0);
                panels.hover(Some(pos), (w, h));
                panels.mouse_down(pos, (w, h));
                panels.mouse_up(pos, (w, h), left);
            }
            panels.take_equipment_change();
            player.set_equipment(&mut renderer, &gpu, &panels.inv.equipment, &panels.stats);
            panels.hover(args.cursor.or(args.clicks.last().copied()), (w, h));
            panels.render_pictures(&mut renderer, &gpu, &player, &world);
            if let (Some(wolves), true) = (&mut wolves, args.target) {
                // The target after the windows opened: the one talked to shows his face in them.
                wolves.set_target(0);
                let view = wolves.target_view(feet);
                panels.set_target(view);
                panels.render_target(&mut renderer, &gpu, wolves, &world);
            }
            if args.panel.iter().any(|p| p == "breath") {
                panels.set_breath(Some((20.0, 30.0)));
            }
            panels.draw(&mut renderer.ui, &gpu, (w, h));
            panels.draw_overlay(&mut renderer.ui, &gpu, (w, h), 0.0, args.panel.iter().any(|p| p == "hint").then_some("Press E to loot"));
            if args.compass {
                panels.found.iter_mut().take(args.discover).for_each(|f| *f = true);
                panels.draw_compass(&mut renderer.ui, &gpu, (w, h), render_camera.pos, render_camera.yaw);
            }
            panels.draw_map(&mut renderer.ui, &gpu, (w, h), render_camera.pos, render_camera.yaw);
            panels.draw_level(&mut renderer.ui, &gpu, (w, h));
            if args.sneak {
                panels.draw_sneak(&mut renderer.ui, &gpu, (w, h));
            }
        }
        // The player previews show the interface too.
        let bar = crate::actionbar::ActionBar::load(&player_mount, &mut renderer.ui, &gpu)?;
        bar.draw(&mut renderer.ui, (w, h), &player, args.panel.iter().any(|p| p == "character"), panels.as_ref().map_or(crate::levels::Progress::new(), |p| p.progress), panels.as_ref().map_or(&[][..], |p| &p.cooldowns[..]), &|i| panels.as_ref().map_or((0, 0.0), |p| p.item_state(i)));
        if args.reticle {
            let art = crate::cursor::Cursors::load(&player_mount, &mut renderer.ui, &gpu)?;
            art.reticle(&mut renderer.ui, (w, h), args.sneak.then(|| args.alert.unwrap_or_else(|| wolves.as_ref().map_or(0.0, |c| c.alert(feet, 1)))));
        }
        // The mouse pointer at `--cursor`, as the game would draw it over what is there.
        if let Some(pos) = args.cursor {
            let art = crate::cursor::Cursors::load(&player_mount, &mut renderer.ui, &gpu)?;
            let over_window = panels.as_mut().and_then(|p| p.cursor_at(pos, (w, h)));
            let kind = over_window.or_else(|| wolves.as_ref().and_then(|c| c.pick(&render_camera, (w, h), pos)).map(|p| p.1)).unwrap_or(crate::cursor::Kind::Point);
            if !panels.as_ref().is_some_and(|p| p.carrying()) {
                art.draw(&mut renderer.ui, kind, pos);
            }
        }
    }
    if let Some((tab, n, steps)) = &args.editor {
        let font = fontdue::Font::from_bytes(player_mount.read(r"Fonts\ARIALN.TTF")?, fontdue::FontSettings::default()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut editor = crate::editor::Editor::new(&mut renderer, &gpu, &font, player_mount.clone())?;
        editor.show(tab, *n);
        editor.mouse_moved((-1.0, -1.0));
        let place = crate::editor::Place { north: -8949.0, west: -132.0, height: 85.0, yaw: 0.0, map: "Azeroth" };
        // Each step is a frame: a click is taken by the frame after it.
        for step in steps {
            editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
            editor.render_preview(&mut renderer, &gpu, &world, render_camera.pos, 0.5);
            editor.update_ghosts(&mut renderer, &gpu, false, 0.5);
            renderer.ui.discard();
            match step.split_once(':') {
                Some(("t", text)) => editor.typed(text),
                // `press:x/y`: the left button goes down over this pixel of the world (selects what is there, or takes hold of the gizmo).
                Some(("press", xy)) => {
                    let (x, y) = xy.split_once('/').context("press:x/y")?;
                    editor.mouse_moved((x.parse()?, y.parse()?));
                    editor.world_press(&|x, z| world.height_at(x, z), &[]);
                }
                // `back:n`: n times Backspace.
                Some(("back", n)) => (0..n.parse::<usize>()?).for_each(|_| editor.key(crate::editor::Key::Backspace)),
                // `hover:x/y`: the pointer is over this pixel of the world (for the ghost of what is put down).
                Some(("hover", xy)) => {
                    let (x, y) = xy.split_once('/').context("hover:x/y")?;
                    let (origin, dir) = crate::propedit::Eye::of(&render_camera, (w, h)).ray(glam::Vec2::new(x.parse()?, y.parse()?));
                    editor.set_hover(world.ray_hit(origin, origin + dir * 400.0));
                }
                Some((x, y)) => {
                    editor.mouse_moved((x.parse()?, y.parse()?));
                    editor.click();
                }
                None => {}
            }
        }
        // The frame that takes the last click, then the one that is shown (after the gallery's pictures were made).
        editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
        editor.render_preview(&mut renderer, &gpu, &world, render_camera.pos, 0.5);
        editor.render_gallery(&mut renderer, &gpu, &world, render_camera.pos, true);
        renderer.ui.discard();
        editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
        editor.render_preview(&mut renderer, &gpu, &world, render_camera.pos, 0.5);
        editor.render_gallery(&mut renderer, &gpu, &world, render_camera.pos, true);
        renderer.ui.discard();
        editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
        editor.update_ghosts(&mut renderer, &gpu, false, 0.5);
    }
    if let Some(first) = args.show.first() {
        let mut tree = crate::uixml::Tree::new(std::rc::Rc::new(if args.glue { crate::uixml::Registry::load_glue(&player_mount)? } else { crate::uixml::Registry::load(&player_mount)? }));
        tree.instantiate(first)?;
        args.show.iter().for_each(|n| tree.show(n, true));
        tree.draw(&mut renderer.ui, &gpu, &player_mount, (w, h));
    }
    if let Some(screen) = &args.glue_screen {
        let dir = args.glue_dir.clone().unwrap_or_else(crate::save::characters_dir);
        let mut glue = crate::glue::Glue::new(player_mount.clone(), &mut renderer, &gpu, dir, Some(screen))?;
        glue.draw(&mut renderer, &gpu, (w, h), 0.1);
        renderer.ui.discard();
        glue.typed(&args.glue_type);
        for &pos in &args.clicks {
            glue.mouse_moved(pos);
            glue.mouse_down(pos);
            glue.mouse_up(pos);
            glue.draw(&mut renderer, &gpu, (w, h), 0.1);
            renderer.ui.discard();
        }
        if let Some(cursor) = args.cursor {
            glue.mouse_moved(cursor);
        }
        glue.draw(&mut renderer, &gpu, (w, h), 0.1);
        log::info!("glue outcome: {:?}", glue.outcome());
    }
    if let Some(progress) = args.loading {
        use crate::loading::Stage::*;
        let mut loading = crate::loading::Loading::load(&player_mount, &args.map, &mut renderer.ui, &gpu)?;
        [Interface, Terrain, Objects].into_iter().for_each(|s| loading.report(s, progress));
        loading.draw(&mut renderer.ui, (w, h));
    }
    renderer.prepare_weather(&gpu, &weather, &render_camera, (w, h));
    renderer.render(&gpu, &view, (w, h), &render_camera, weather.clock, world.tiles());

    // Read back (rows must be padded to 256 bytes).
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device.poll(wgpu::Maintain::Wait);
    rx.recv()??;

    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        pixels.extend_from_slice(&data[row * padded as usize..][..(w * 4) as usize]);
    }
    drop(data);
    buffer.unmap();

    let file = std::fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&pixels)?;
    log::info!("wrote {}", out.display());
    Ok(())
}
