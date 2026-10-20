//! Draws the weather of `weather.rs` with the vertex programs of `weather.wgsl`.

use crate::models::{ModelCache, RawTexture};
use crate::renderer::{Atmosphere, Gpu, DEPTH_FORMAT};
use crate::skygpu::texture_view;
use crate::weather::{Kind, Particle, Weather, PUFF_SIZE};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::sync::Arc;

/// The most particles and splashes drawn.
const MOST_PARTICLES: usize = 90_000;
const MOST_SPLASHES: usize = 8_000;
const MOST_PUFFS: usize = 96;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    clock: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    tint: [f32; 4],
    view: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MistVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

/// The pictures of the weather, from the game's files.
pub struct WeatherAssets {
    rain: Arc<RawTexture>,
    splash: Arc<RawTexture>,
    flake: Arc<RawTexture>,
    mist: Arc<RawTexture>,
    grain: Arc<RawTexture>,
}

impl WeatherAssets {
    pub fn load(mount: &Mount) -> Result<Self> {
        let cache = ModelCache::default();
        let texture = |name: &str| cache.texture(mount, name).map(|(_, t)| t).with_context(|| format!("no weather texture {name}"));
        Ok(WeatherAssets {
            rain: texture(r"textures\Weather\RainDrop01.blp")?,
            splash: texture(r"textures\Weather\RainDropSplash01.blp")?,
            flake: texture(r"textures\Weather\Snowflake01.blp")?,
            mist: texture(r"textures\Weather\SnowMist01.blp")?,
            grain: texture(r"textures\Weather\WeatherMistGrainy01.blp")?,
        })
    }
}

pub struct WeatherGpu {
    rain: wgpu::RenderPipeline,
    snow: wgpu::RenderPipeline,
    sand: wgpu::RenderPipeline,
    splash: wgpu::RenderPipeline,
    mist: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    pictures: [wgpu::BindGroup; 5],
    particles: wgpu::Buffer,
    splashes: wgpu::Buffer,
    puffs: wgpu::Buffer,
    kind: Option<Kind>,
    particle_count: u32,
    splash_count: u32,
    puff_count: u32,
    clock: f32,
}

const RAIN: usize = 0;
const SPLASH: usize = 1;
const FLAKE: usize = 2;
const MIST: usize = 3;
const GRAIN: usize = 4;

impl WeatherGpu {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, assets: &WeatherAssets) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("weather"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/weather.wgsl").into()) });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("weather uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let picture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("weather picture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let picture = |tex: &RawTexture| {
            let view = texture_view(gpu, tex);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("weather picture"),
                layout: &picture_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
            })
        };
        let pictures = [picture(&assets.rain), picture(&assets.splash), picture(&assets.flake), picture(&assets.mist), picture(&assets.grain)];

        let layout = |groups: &[&wgpu::BindGroupLayout]| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("weather pipeline layout"), bind_group_layouts: groups, push_constant_ranges: &[] });
        let untextured = layout(&[&uniform_layout]);
        let textured = layout(&[&uniform_layout, &picture_layout]);
        let instance = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Float32];
        let puff = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let pipeline = |label: &str, layout: &wgpu::PipelineLayout, vertex: &str, fragment: &str, buffer: wgpu::VertexBufferLayout| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vertex), buffers: &[buffer], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: false, depth_compare: wgpu::CompareFunction::LessEqual, stencil: Default::default(), bias: Default::default() }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let per_instance = || wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Particle>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &instance };
        let per_vertex = || wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<MistVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &puff };

        let buffer = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("weather uniform"), size: std::mem::size_of::<Uniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("weather uniform"), layout: &uniform_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }] });
        WeatherGpu {
            rain: pipeline("rain", &textured, "vs_rain", "fs_tex", per_instance()),
            snow: pipeline("snow", &textured, "vs_snow", "fs_tex", per_instance()),
            sand: pipeline("sand", &untextured, "vs_sand", "fs_flat", per_instance()),
            splash: pipeline("splashes", &textured, "vs_splash", "fs_tex", per_instance()),
            mist: pipeline("mist", &textured, "vs_mist", "fs_tex", per_vertex()),
            uniform,
            uniform_bg,
            pictures,
            particles: buffer("weather particles", MOST_PARTICLES * std::mem::size_of::<Particle>()),
            splashes: buffer("weather splashes", MOST_SPLASHES * std::mem::size_of::<Particle>()),
            puffs: buffer("weather mist", MOST_PUFFS * 6 * std::mem::size_of::<MistVertex>()),
            kind: None,
            particle_count: 0,
            splash_count: 0,
            puff_count: 0,
            clock: 0.0,
        }
    }

    /// Sends this frame's weather to the card: the particles as they are, the puffs of mist as pictures turned to the camera.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(&mut self, gpu: &Gpu, weather: &Weather, view_proj: &Mat4, eye: Vec3, size: (u32, u32), a: &Atmosphere) {
        self.kind = weather.kind();
        self.clock = weather.clock;
        let inverse = view_proj.inverse();
        // The camera's right and up, from where the corners of the view land at the far plane.
        let screen = |x: f32, y: f32| {
            let p = inverse * glam::Vec4::new(x, y, 0.5, 1.0);
            p.truncate() / p.w
        };
        let right = (screen(1.0, 0.0) - screen(-1.0, 0.0)).normalize_or_zero();
        let up = (screen(0.0, 1.0) - screen(0.0, -1.0)).normalize_or_zero();

        let ambient = a.ambient.max(a.sun_color * 0.4).clamp(Vec3::splat(0.15), Vec3::ONE);
        let tint = match weather.kind() {
            Some(Kind::Sand) => (a.fog_color * 1.2).clamp(Vec3::ZERO, Vec3::ONE),
            _ => ambient,
        };
        let uniform = Uniform {
            view_proj: view_proj.to_cols_array_2d(),
            eye: eye.extend(0.0).to_array(),
            clock: [weather.clock, size.1 as f32 / 768.0, size.0 as f32 * 0.0025, 0.0],
            fog_color: a.fog_color.extend(1.0).to_array(),
            fog: [a.fog_start, a.fog_end, 0.0, 0.0],
            tint: tint.extend(1.0).to_array(),
            view: [size.0 as f32, size.1 as f32, 0.0, 0.0],
            right: right.extend(0.0).to_array(),
            up: up.extend(0.0).to_array(),
        };
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));

        let n = weather.particles.len().min(MOST_PARTICLES);
        self.particle_count = if weather.kind().is_some() { n as u32 } else { 0 };
        gpu.queue.write_buffer(&self.particles, 0, bytemuck::cast_slice(&weather.particles[..n]));
        // Only the splashes that have begun (the drops that land in the next moments are known already).
        let splashes: Vec<Particle> = weather.splashes.iter().filter(|s| s.t0 <= weather.clock).take(MOST_SPLASHES).copied().collect();
        self.splash_count = splashes.len() as u32;
        gpu.queue.write_buffer(&self.splashes, 0, bytemuck::cast_slice(&splashes));

        // A puff fades in and out over a quarter of its life and fades out as it comes within 6 to 18 yards of the camera (FUN_0067add0).
        let mut vertices = Vec::with_capacity(weather.puffs.len().min(MOST_PUFFS) * 6);
        for puff in weather.puffs.iter().take(MOST_PUFFS) {
            let age = (weather.clock - puff.born) / puff.life;
            let pos = puff.pos + puff.vel * (weather.clock - puff.born);
            let near = ((pos.distance(eye) - 6.0) / 12.0).clamp(0.0, 1.0);
            let alpha = (age / 0.25).min((1.0 - age) / 0.25).clamp(0.0, 1.0) * near * 0.8;
            let (s, c) = (puff.spin * age).sin_cos();
            let (r, u) = (right * c + up * s, up * c - right * s);
            let half = PUFF_SIZE * 0.5;
            let color = [tint.x, tint.y, tint.z, alpha];
            let at = |x: f32, y: f32| MistVertex { pos: (pos + (r * x + u * y) * half).to_array(), uv: [x * 0.5 + 0.5, 0.5 - y * 0.5], color };
            vertices.extend([at(-1.0, -1.0), at(1.0, -1.0), at(-1.0, 1.0), at(1.0, 1.0), at(-1.0, 1.0), at(1.0, -1.0)]);
        }
        self.puff_count = vertices.len() as u32;
        gpu.queue.write_buffer(&self.puffs, 0, bytemuck::cast_slice(&vertices));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(kind) = self.kind else { return };
        pass.set_bind_group(0, &self.uniform_bg, &[]);
        if self.puff_count > 0 {
            pass.set_pipeline(&self.mist);
            pass.set_bind_group(1, &self.pictures[if kind == Kind::Sand { GRAIN } else { MIST }], &[]);
            pass.set_vertex_buffer(0, self.puffs.slice(..));
            pass.draw(0..self.puff_count, 0..1);
        }
        if self.particle_count > 0 {
            pass.set_vertex_buffer(0, self.particles.slice(..));
            match kind {
                Kind::Rain => {
                    pass.set_pipeline(&self.rain);
                    pass.set_bind_group(1, &self.pictures[RAIN], &[]);
                    pass.draw(0..3, 0..self.particle_count);
                }
                Kind::Snow => {
                    pass.set_pipeline(&self.snow);
                    pass.set_bind_group(1, &self.pictures[FLAKE], &[]);
                    pass.draw(0..6, 0..self.particle_count);
                }
                Kind::Sand => {
                    pass.set_pipeline(&self.sand);
                    pass.draw(0..6, 0..self.particle_count);
                }
            }
        }
        if kind == Kind::Rain && self.splash_count > 0 {
            pass.set_pipeline(&self.splash);
            pass.set_bind_group(1, &self.pictures[SPLASH], &[]);
            pass.set_vertex_buffer(0, self.splashes.slice(..));
            pass.draw(0..3, 0..self.splash_count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_weather_pictures_are_in_the_game_files() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let assets = WeatherAssets::load(&mount).expect("weather pictures load");
        // The drop is a thin streak, the splash atlas is four by four pictures of equal size.
        assert!(assets.rain.height > assets.rain.width * 4);
        assert_eq!(assets.splash.width, assets.splash.height);
    }
}
