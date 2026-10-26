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

mod probes;
mod scenes;

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
    probes::run(args, &world, &camera, z_given);
    probes::walk(args, &mut camera, &world);

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
    scenes::view(args, &player_mount, &mut renderer, &gpu, &world, &mut render_camera, camera, w, h)?;
    scenes::editor(args, &player_mount, &mut renderer, &gpu, &world, &render_camera, w, h)?;
    scenes::show(args, &player_mount, &mut renderer, &gpu, w, h)?;
    scenes::glue_screen(args, &player_mount, &mut renderer, &gpu, w, h)?;
    scenes::loading(args, &player_mount, &mut renderer, &gpu, w, h)?;
    renderer.prepare_weather(&gpu, &weather, &render_camera, (w, h));
    renderer.render(&gpu, &view, (w, h), &render_camera, weather.clock, world.tiles());

    save_png(&gpu, &target, (w, h), out)
}

/// Copy the picture back from the GPU and write it as a PNG.
fn save_png(gpu: &Gpu, target: &wgpu::Texture, (w, h): (u32, u32), out: &Path) -> Result<()> {
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
