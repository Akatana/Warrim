//! wgpu renderer. Colours are authored in gamma space like the original game,
//! so everything uses non-sRGB formats and no linear conversion.

use crate::camera::Camera;
use crate::coords::{tile_of, tile_origin, TileCoord};
use crate::models::{ModelCpu, ModelVertex, RawTexture, WmoCpu};
use crate::ui::{Quad, Ui, FULL_UV};
use crate::terrain::{GroundTexture, TerrainVertex, TileCpu, WaterVertex, GROUND_TEX_MIPS, GROUND_TEX_SIZE};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use wowfmt::m2::Blend;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    fog_color: [f32; 4],
    fog_params: [f32; 4],
    // Sky pass only.
    inv_view_proj: [[f32; 4]; 4],
    sky_top: [f32; 4],
    sky_mid: [f32; 4],
    // Sun shadows: the sun's view and projection, and the numbers the lookup needs (see globals.wgsl).
    light_view_proj: [[f32; 4]; 4],
    shadow: [f32; 4],
}

/// The sun's shadow map: its size in texels, the yards it covers each way around the camera, and the yards of depth it spans.
const SHADOW_SIZE: u32 = 2048;
const SHADOW_RADIUS: f32 = 64.0;
const SHADOW_DEPTH: f32 = 400.0;

/// The projection of the sun's shadow map (`sun_dir`: the way its light goes), for geometry relative to the camera at `cam`. The map's
/// edges stay on whole texels of the world, so shadows do not crawl when the camera moves.
fn light_view_proj(cam: Vec3, sun_dir: Vec3) -> Mat4 {
    let up = Vec3::Y;
    let at = Mat4::look_to_rh(Vec3::ZERO, sun_dir, up).transform_point3(cam);
    let texel = 2.0 * SHADOW_RADIUS / SHADOW_SIZE as f32;
    let (dx, dy) = ((at.x / texel).round() * texel - at.x, (at.y / texel).round() * texel - at.y);
    let r = SHADOW_RADIUS;
    Mat4::orthographic_rh(dx - r, dx + r, dy - r, dy + r, 0.0, SHADOW_DEPTH) * Mat4::look_to_rh(-sun_dir * SHADOW_DEPTH / 2.0, sun_dir, up)
}

/// A box in tile-local space: (min, max).
type Bounds = (Vec3, Vec3);

const NO_BOUNDS: Bounds = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));

fn grow(b: &mut Bounds, p: Vec3) {
    *b = (b.0.min(p), b.1.max(p));
}

/// The box of `local` after `matrix`, added to `b`.
fn grow_transformed(b: &mut Bounds, matrix: &Mat4, local: Bounds) {
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { local.0.x } else { local.1.x },
            if i & 2 == 0 { local.0.y } else { local.1.y },
            if i & 4 == 0 { local.0.z } else { local.1.z },
        );
        grow(b, matrix.transform_point3(corner));
    }
}

/// The six planes of a view-projection (clip depth 0..1) of geometry relative to the camera at `cam`: what is outside them is not drawn.
struct Frustum {
    planes: [glam::Vec4; 6],
    cam: Vec3,
    /// Props further away than this are not drawn (the video options' environment detail); the shadows' view has no limit.
    reach: f32,
}

impl Frustum {
    fn new(view_proj: &Mat4, cam: Vec3) -> Self {
        let r = [view_proj.row(0), view_proj.row(1), view_proj.row(2), view_proj.row(3)];
        Self { planes: [r[3] + r[0], r[3] - r[0], r[3] + r[1], r[3] - r[1], r[2], r[3] - r[2]], cam, reach: f32::INFINITY }
    }

    /// Whether the box `b` is within `reach` of the camera.
    fn near(&self, origin: (f64, f64), b: Bounds) -> bool {
        let o = Vec3::new((origin.0 - self.cam.x as f64) as f32, -self.cam.y, (origin.1 - self.cam.z as f64) as f32);
        Vec3::ZERO.clamp(b.0 + o, b.1 + o).length() <= self.reach
    }

    /// Whether the box `b`, in the space of the tile at `origin`, may touch the view.
    fn sees(&self, origin: (f64, f64), b: Bounds) -> bool {
        let o = Vec3::new((origin.0 - self.cam.x as f64) as f32, -self.cam.y, (origin.1 - self.cam.z as f64) as f32);
        let (min, max) = (b.0 + o, b.1 + o);

        self.planes.iter().all(|p| {
            let n = p.truncate();
            n.dot(Vec3::select(n.cmpge(Vec3::ZERO), max, min)) + p.w >= 0.0
        })
    }
}

pub struct Atmosphere {
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub ambient: Vec3,
    pub fog_color: Vec3,
    pub sky_top: Vec3,
    pub sky_mid: Vec3,
    pub fog_start: f32,
    pub fog_end: f32,
    /// The sky colours between the middle and the horizon (light bands 4, 5 and 6); the horizon itself is the fog colour.
    pub sky_low: [Vec3; 3],
    /// The tint of sun and moon (band 9) and the colours of the clouds (bands 10 and 11): lit by the sun, and in shade.
    pub celestial: Vec3,
    pub cloud_lit: Vec3,
    pub cloud_shade: Vec3,
    /// The colour added to every cloud (band 12).
    pub cloud_base: Vec3,
    /// The share of the sky the clouds cover (float band 3), and how much the sky glows at sunrise and sunset (`LightParams.highlightSky`, 0 to 1).
    pub cloud_cover: f32,
    pub highlight: f32,
    /// How much the weather hides the sun, the moon and the stars, 0 to 1.
    pub storm: f32,
    /// The time of day in hours, 0 to 24.
    pub hour: f32,
}

impl Atmosphere {
    pub fn daytime(view_distance: f32) -> Self {
        Self {
            sun_dir: Vec3::new(-0.45, -0.75, -0.35).normalize(),
            sun_color: Vec3::new(0.78, 0.72, 0.60),
            ambient: Vec3::new(0.52, 0.52, 0.56),
            fog_color: Vec3::new(0.62, 0.74, 0.88),
            sky_top: Vec3::new(0.25, 0.45, 0.80),
            sky_mid: Vec3::new(0.45, 0.65, 0.90),
            fog_start: view_distance * 0.45,
            fog_end: view_distance,
            sky_low: [Vec3::new(0.55, 0.72, 0.92), Vec3::new(0.6, 0.76, 0.92), Vec3::new(0.62, 0.74, 0.88)],
            celestial: Vec3::new(1.0, 0.97, 0.87),
            cloud_lit: Vec3::new(1.0, 0.78, 0.54),
            cloud_shade: Vec3::new(0.17, 0.41, 0.52),
            cloud_base: Vec3::ZERO,
            cloud_cover: 0.5,
            highlight: 0.0,
            storm: 0.0,
            hour: 12.0,
        }
    }
}

/// GPU resources of one terrain tile.
pub struct TileGpu {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    water: Option<(wgpu::Buffer, u32)>,
    doodads: Vec<DoodadGroup>,
    /// The models of the tile whose batches change their look as time goes: the model, its tracks and what each batch follows.
    animated: Vec<(Arc<ModelGpu>, Arc<crate::anim::LookTracks>, Vec<crate::anim::BatchRefs>)>,
    wmos: Vec<WmoGpu>,
    /// The shadows baked into the ground (`TileCpu::shadow`).
    shadow: Vec<u8>,
    coord: TileCoord,
    origin: (f64, f64),
    /// The terrain and water of the tile.
    bounds: Bounds,
}

impl TileGpu {
    /// Whether a point (render space) is inside one of the tile's buildings (see `wmovis::indoors`).
    pub fn indoors(&self, at: Vec3) -> bool {
        let local = Vec3::new((at.x as f64 - self.origin.0) as f32, at.y, (at.z as f64 - self.origin.1) as f32);
        self.wmos.iter().any(|w| {
            let r = w.matrix.inverse().transform_point3(local);
            crate::wmovis::indoors(&w.wmo, Vec3::new(r.x, -r.z, r.y))
        })
    }

    /// The `WMOAreaTable` numbers (building, group) of the room of one of the tile's buildings a point (render space) is in.
    pub fn room_at(&self, at: Vec3) -> Option<(u32, u32)> {
        let local = Vec3::new((at.x as f64 - self.origin.0) as f32, at.y, (at.z as f64 - self.origin.1) as f32);
        self.wmos.iter().find_map(|w| {
            let r = w.matrix.inverse().transform_point3(local);
            crate::wmovis::room_at(&w.wmo, Vec3::new(r.x, -r.z, r.y))
        })
    }
}

struct BatchGpu {
    first: u32,
    count: u32,
    bind_group: wgpu::BindGroup,
    blend: Blend,
    /// The batch's uniform (`Params` of the model shader), and its fixed first four numbers: written again when the batch's look animates.
    params: wgpu::Buffer,
    base: [f32; 4],
    /// How a building's batch is lit: `Baked::code`, 1 for a window (the spare numbers of the texture matrix rows), and the colour it glows with.
    lit: [f32; 5],
}

/// An uploaded model, shared by every tile that places it.
pub struct ModelGpu {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    batches: Vec<BatchGpu>,
    /// Model-space box of the vertices.
    bounds: Bounds,
}

impl ModelGpu {
    fn has(&self, blends: &[Blend]) -> bool {
        self.batches.iter().any(|b| blends.contains(&b.blend))
    }

    /// Give the batches the looks (colour, opacity, moved texture) of this moment.
    fn set_looks(&self, gpu: &Gpu, looks: &[crate::anim::BatchLook]) {
        for (b, look) in self.batches.iter().zip(looks) {
            gpu.queue.write_buffer(&b.params, 0, bytemuck::cast_slice(&batch_params(b.base, b.lit, look)));
        }
    }
}

/// The batch uniform: the fixed numbers, the colour and opacity, and two rows of the texture matrix (with how the batch is lit behind each).
fn batch_params(base: [f32; 4], lit: [f32; 5], look: &crate::anim::BatchLook) -> [f32; 20] {
    let [r, g, b] = look.tint;
    [
        base[0], base[1], base[2], base[3], r, g, b, look.opacity, look.uv[0][0], look.uv[0][1], look.uv[0][2], lit[0], look.uv[1][0], look.uv[1][1], look.uv[1][2], lit[1], lit[2], lit[3], lit[4], 0.0,
    ]
}

/// What one placed model gives the model shader: its matrix (16 floats), then the ambient light of the room it stands in (rgb and 1, or zeros for the open
/// air), the light from above (rgb, and how much of its direction is the sun's), and the four lamps that light it, each two words: the colour (rgb, ten bits
/// each, 0 to 2) and the direction toward the lamp (xyz, ten bits each, signed).
const INSTANCE_FLOATS: usize = 24 + 4 * 2;
const INSTANCE_BYTES: u64 = (INSTANCE_FLOATS * 4) as u64;

fn instance_floats(matrix: &Mat4, light: Option<crate::models::InteriorLight>) -> [f32; INSTANCE_FLOATS] {
    let mut out = [0.0; INSTANCE_FLOATS];
    out[..16].copy_from_slice(&matrix.to_cols_array());
    if let Some(light) = light {
        out[16..19].copy_from_slice(&light.ambient);
        out[19] = 1.0;
        out[20..23].copy_from_slice(&light.diffuse);
        out[23] = light.sun_mix;
        for (i, lamp) in light.lamps.iter().enumerate() {
            let unorm = |c: f32| (c / 2.0 * 1023.0).round().clamp(0.0, 1023.0) as u32;
            let snorm = |c: f32| ((c * 511.0).round().clamp(-511.0, 511.0) as i32 as u32) & 1023;
            let [r, g, b] = lamp.color;
            let [x, y, z] = lamp.dir;
            out[24 + i * 2] = f32::from_bits(unorm(r) | unorm(g) << 10 | unorm(b) << 20);
            out[25 + i * 2] = f32::from_bits(snorm(x) | snorm(y) << 10 | snorm(z) << 20);
        }
    }
    out
}

/// A movable model that is not part of a tile: the player, equipped items.
struct EntityGpu {
    /// How it is lit if it stands in a room.
    light: Option<crate::models::InteriorLight>,
    /// It is lit by the room it moves through (`add_room_lit_entity`): `light` follows `room_target`, found at `room_sampled`, by easing.
    room_lit: bool,
    room_target: Option<crate::models::InteriorLight>,
    room_sampled: Vec3,
    model: Arc<ModelGpu>,
    instance: wgpu::Buffer,
    /// Render-space position and orientation (rotation, no translation).
    pos: Vec3,
    orient: Mat4,
    visible: bool,
    /// Opacity of the whole model: below 1 its opaque batches are drawn blended (a sneaking character).
    fade: f32,
}

/// A placed building. `visible[g]` says whether group `g` is drawn this frame.
struct WmoGpu {
    wmo: Arc<WmoCpu>,
    matrix: Mat4,
    /// Its lamps, where they are in the tile.
    lamps: Vec<crate::models::Lamp>,
    groups: Vec<Option<DoodadGroup>>,
    visible: Vec<Cell<bool>>,
}

struct DoodadGroup {
    model: Arc<ModelGpu>,
    instances: wgpu::Buffer,
    count: u32,
    /// Tile-local box of all its instances.
    bounds: Bounds,
}

/// Doodads are only drawn for tiles within this many tiles of the camera's tile.
const DOODAD_TILE_RADIUS: i32 = 1;

pub struct Renderer {
    pub atmosphere: Atmosphere,
    /// The video options: how far props are drawn (yards) and the screen's gamma (-0.5 to 0.5).
    pub prop_reach: f32,
    pub gamma: f32,
    /// Whether the sun casts shadows while it is up.
    pub shadows: bool,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    /// Shadow pass: the sun's globals (a second copy of `globals_buf`), the map it draws into, and the pipelines of terrain and models.
    shadow_buf: wgpu::Buffer,
    shadow_bg: wgpu::BindGroup,
    shadow_map: wgpu::TextureView,
    shadow_terrain: wgpu::RenderPipeline,
    shadow_model: wgpu::RenderPipeline,
    tile_layout: wgpu::BindGroupLayout,
    /// What the pictures of models on no tile (the character list) stand on: a tile's uniform and bind group with nothing in them.
    stage: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    terrain_pipeline: wgpu::RenderPipeline,
    ground_sampler: wgpu::Sampler,
    alpha_sampler: wgpu::Sampler,
    water_layout: wgpu::BindGroupLayout,
    water_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    /// The sky of the original (dome, stars, sun, moon, clouds); until it is set the plain gradient shows.
    sky: Option<crate::skygpu::SkyGpu>,
    /// Rain, snow and sand; set once the pictures are loaded and filled each frame by `prepare_weather`.
    weather: Option<crate::weathergpu::WeatherGpu>,
    water_bg: Option<wgpu::BindGroup>,
    model_layout: wgpu::BindGroupLayout,
    /// opaque/alpha-key, alpha blended, additive
    model_pipelines: [wgpu::RenderPipeline; 5],
    model_samplers: [wgpu::Sampler; 2], // clamp, repeat
    models: HashMap<String, Arc<ModelGpu>>,
    model_textures: HashMap<String, wgpu::TextureView>,
    grey: wgpu::TextureView,
    depth: Option<(wgpu::TextureView, (u32, u32))>,
    entities: Vec<EntityGpu>,
    /// The time of the last frame, for what eases.
    room_clock: f32,
    /// Entities given back (`remove_entity`), to be used again by the next `add_entity`.
    free: Vec<usize>,
    /// The interface, drawn over the finished frame.
    pub ui: Ui,
    format: wgpu::TextureFormat,
    offscreens: Vec<Offscreen>,
}

/// A picture models are drawn into for the interface (the paper doll and the portrait).
struct Offscreen {
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// Upload equally sized (256x256, full mip chain) textures as one 2D array.
fn create_texture_array(gpu: &Gpu, textures: &[Arc<GroundTexture>], label: &str) -> wgpu::TextureView {
    let layers = textures.len().max(1) as u32;
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: GROUND_TEX_SIZE, height: GROUND_TEX_SIZE, depth_or_array_layers: layers },
        mip_level_count: GROUND_TEX_MIPS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, t) in textures.iter().enumerate() {
        for (mip, data) in t.mips.iter().enumerate() {
            let s = (GROUND_TEX_SIZE >> mip).max(1);
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: mip as u32,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 },
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(s * 4), rows_per_image: Some(s) },
                wgpu::Extent3d { width: s, height: s, depth_or_array_layers: 1 },
            );
        }
    }
    tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

impl Renderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, atmosphere: Atmosphere) -> Self {
        let anisotropy = crate::video::VideoSettings::load().anisotropy_clamp();
        let device = &gpu.device;

        let globals_entry = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[
                globals_entry(wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let globals_buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<Globals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let globals_buf = globals_buffer("globals");
        let shadow_buf = globals_buffer("shadow globals");
        let shadow_map = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("shadow map"),
                size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_map) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            ],
        });
        // The shadow pass cannot bind the map it draws into, so it has globals of its own.
        let shadow_globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow globals layout"),
            entries: &[globals_entry(wgpu::ShaderStages::VERTEX)],
        });
        let shadow_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow globals"),
            layout: &shadow_globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_buf.as_entire_binding() }],
        });

        let array_tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                array_tex(1),
                sampler(2),
                array_tex(3),
                sampler(4),
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/globals.wgsl"), include_str!("shaders/terrain.wgsl")).into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain pipeline layout"),
            bind_group_layouts: &[&globals_layout, &tile_layout],
            push_constant_ranges: &[],
        });
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x2, 4 => Uint32x4, 5 => Uint32
        ];
        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // Water: group 2 holds the animated liquid textures.
        let water_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water layout"),
            entries: &[array_tex(0), sampler(1)],
        });
        let water_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/water.wgsl").into()),
        });
        let water_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water pipeline layout"),
            bind_group_layouts: &[&globals_layout, &tile_layout, &water_layout],
            push_constant_ranges: &[],
        });
        let water_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Uint32];
        let water_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&water_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &water_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WaterVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &water_attrs,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &water_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sky.wgsl").into()),
        });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sky pipeline layout"),
                bind_group_layouts: &[&globals_layout],
                push_constant_ranges: &[],
            })),
            vertex: wgpu::VertexState {
                module: &sky_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &sky_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // Models: group 2 = one batch (texture + sampler + params).
        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model batch layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                sampler(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let model_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("model"),
            source: wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/globals.wgsl"), include_str!("shaders/model.wgsl")).into()),
        });
        let model_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("model pipeline layout"),
            bind_group_layouts: &[&globals_layout, &tile_layout, &model_layout],
            push_constant_ranges: &[],
        });
        let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 7 => Unorm8x4];
        let instance_attrs = wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Uint32x4, 11 => Uint32x4];
        let make_model_pipeline = |label: &str, blend: Option<wgpu::BlendState>, depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&model_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &model_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[
                        wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<ModelVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &vertex_attrs,
                        },
                        wgpu::VertexBufferLayout {
                            array_stride: INSTANCE_BYTES,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &instance_attrs,
                        },
                    ],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &model_shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: depth_write,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });
        let shadow_depth = Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: wgpu::DepthBiasState { constant: 0, slope_scale: 2.0, clamp: 0.0 },
        });
        let shadow_terrain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow terrain"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow terrain layout"),
                bind_group_layouts: &[&shadow_globals_layout, &tile_layout],
                push_constant_ranges: &[],
            })),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_terrain"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                }],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: Default::default(),
            depth_stencil: shadow_depth.clone(),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let shadow_model = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow model"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow model layout"),
                bind_group_layouts: &[&shadow_globals_layout, &tile_layout, &model_layout],
                push_constant_ranges: &[],
            })),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_model"),
                buffers: &[
                    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<ModelVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vertex_attrs },
                    wgpu::VertexBufferLayout { array_stride: INSTANCE_BYTES, step_mode: wgpu::VertexStepMode::Instance, attributes: &instance_attrs },
                ],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState { module: &shadow_shader, entry_point: Some("fs_model"), targets: &[], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: shadow_depth,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let modulate = |dst_factor| wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::OVER,
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let model_pipelines = [
            make_model_pipeline("model opaque", None, true),
            make_model_pipeline("model alpha", Some(wgpu::BlendState::ALPHA_BLENDING), false),
            make_model_pipeline("model additive", Some(additive), false),
            // Pictures that multiply what is behind them: once, and twice over.
            make_model_pipeline("model modulate", Some(modulate(wgpu::BlendFactor::Zero)), false),
            make_model_pipeline("model modulate 2x", Some(modulate(wgpu::BlendFactor::Src)), false),
        ];
        let model_sampler = |mode: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("model sampler"),
                address_mode_u: mode,
                address_mode_v: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                anisotropy_clamp: anisotropy,
                ..Default::default()
            })
        };
        let model_samplers = [model_sampler(wgpu::AddressMode::ClampToEdge), model_sampler(wgpu::AddressMode::Repeat)];
        let grey = {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("grey"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[128, 128, 128, 255],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            tex.create_view(&Default::default())
        };

        let ground_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ground sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: anisotropy,
            ..Default::default()
        });
        let alpha_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("alpha sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            atmosphere,
            prop_reach: f32::INFINITY,
            gamma: 0.0,
            shadows: true,
            globals_buf,
            globals_bg,
            shadow_buf,
            shadow_bg,
            shadow_map,
            shadow_terrain,
            shadow_model,
            tile_layout,
            stage: None,
            terrain_pipeline,
            ground_sampler,
            alpha_sampler,
            water_layout,
            water_pipeline,
            sky_pipeline,
            sky: None,
            weather: None,
            water_bg: None,
            model_layout,
            model_pipelines,
            model_samplers,
            models: HashMap::new(),
            model_textures: HashMap::new(),
            grey,
            depth: None,
            ui: Ui::new(gpu, format),
            format,
            offscreens: Vec::new(),
            entities: Vec::new(),
            room_clock: 0.0,
            free: Vec::new(),
        }
    }

    /// Provide the liquid animation frames (`kind * 30 + frame`, see `terrain::load_water_textures`).
    pub fn set_water_textures(&mut self, gpu: &Gpu, textures: &[Arc<GroundTexture>]) {
        let anisotropy = crate::video::VideoSettings::load().anisotropy_clamp();
        let view = create_texture_array(gpu, textures, "water textures");
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("water sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: anisotropy.min(4),
            ..Default::default()
        });
        self.water_bg = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water bind group"),
            layout: &self.water_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }));
    }

    /// Add a model that moves on its own. Its vertex buffer can be rewritten each frame
    /// (`update_entity_vertices`); it starts hidden.
    pub fn add_entity(&mut self, gpu: &Gpu, model: &ModelCpu) -> usize {
        // Every entity gets vertex buffers of its own: they are re-skinned per entity (two wolves must not share a pose).
        self.models.remove(&model.key);
        let model = self.model_gpu(gpu, model);
        let instance = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("entity instance"),
            size: INSTANCE_BYTES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity = EntityGpu { light: None, room_lit: false, room_target: None, room_sampled: Vec3::splat(f32::MAX), model, instance, pos: Vec3::ZERO, orient: Mat4::IDENTITY, visible: false, fade: 1.0 };
        match self.free.pop() {
            Some(id) => {
                self.entities[id] = entity;
                id
            }
            None => {
                self.entities.push(entity);
                self.entities.len() - 1
            }
        }
    }

    /// Add a model that moves on its own and is lit by the rooms of buildings it moves through (a creature, a character): see `ease_room_lights`.
    pub fn add_room_lit_entity(&mut self, gpu: &Gpu, model: &ModelCpu) -> usize {
        let id = self.add_entity(gpu, model);
        self.entities[id].room_lit = true;
        id
    }

    /// Light the entities that move through rooms as WoW.exe does (FUN_0069e770, FUN_0069e4c0): the light of the place under them is found when they
    /// have moved a little, and the ambient light they have eases toward it at 2 per second (all of it in half a second), so that walking through a
    /// door does not change them with a jump. Out of a room it eases back to the light of the open air, and then it is the open air's.
    fn ease_room_lights(&mut self, tiles: &[&TileGpu], time: f32) {
        let dt = (time - self.room_clock).clamp(0.0, 0.25);
        self.room_clock = time;
        let outdoors = crate::models::InteriorLight::outdoors(self.atmosphere.ambient, self.atmosphere.sun_color);
        for e in self.entities.iter_mut().filter(|e| e.room_lit && e.visible) {
            if e.pos.distance(e.room_sampled) > 0.2 {
                e.room_sampled = e.pos;
                let tile = tile_of(e.pos.x, e.pos.z);
                e.room_target = tiles.iter().find(|t| t.coord == tile).and_then(|t| {
                    let local = Vec3::new((e.pos.x as f64 - t.origin.0) as f32, e.pos.y, (e.pos.z as f64 - t.origin.1) as f32);
                    let intensity = if crate::terrain::ground_shadowed(&t.shadow, local.x, local.z) { 0.5 } else { 2.5 };
                    let (mut light, w) = crate::models::room_light_at(t.wmos.iter().map(|w| (&*w.wmo, &w.matrix)), local, intensity)?;
                    light.lamps = crate::models::lamp_lights(&t.wmos[w].lamps, local);
                    Some(light)
                });
            }
            if e.light.is_none() && e.room_target.is_none() {
                continue;
            }
            let to = e.room_target.unwrap_or(outdoors);
            let eased = e.light.unwrap_or(outdoors).eased(to, 2.0 * dt);
            e.light = if eased == to && e.room_target.is_none() { None } else { Some(eased) };
        }
    }

    /// Give an entity back: it is hidden, and its place (with its buffers) goes to the next one added. Nothing may use `id` after this.
    pub fn remove_entity(&mut self, id: usize) {
        self.entities[id].visible = false;
        self.free.push(id);
    }

    /// Place an entity: `pos` in render space, `orient` its rotation (and scale).
    pub fn set_entity(&mut self, id: usize, pos: Vec3, orient: Mat4, visible: bool) {
        let e = &mut self.entities[id];
        (e.pos, e.orient, e.visible) = (pos, orient, visible);
    }

    /// Give the batches of an entity the looks of this moment (see `anim::LookTracks`).
    pub fn set_entity_looks(&self, gpu: &Gpu, id: usize, looks: &[crate::anim::BatchLook]) {
        self.entities[id].model.set_looks(gpu, looks);
    }

    /// Make the entity see-through (`fade` 1 is solid). Replaces the looks of its batches, so call it after `set_entity_looks`.
    pub fn set_entity_fade(&mut self, gpu: &Gpu, id: usize, fade: f32) {
        let e = &mut self.entities[id];
        e.fade = fade;
        let look = crate::anim::BatchLook { opacity: fade, ..crate::anim::BatchLook::STILL };
        for b in &e.model.batches {
            // A negative alpha test tells the shader to ignore the texture's alpha (opaque batches have none to speak of).
            let base = if fade < 1.0 && b.blend == Blend::Opaque { [-1.0, b.base[1], b.base[2], b.base[3]] } else { b.base };
            gpu.queue.write_buffer(&b.params, 0, bytemuck::cast_slice(&batch_params(base, b.lit, &look)));
        }
    }

    /// Light the entity as a model in a room of a building (`None`: as in the open air).
    pub fn set_entity_light(&mut self, id: usize, light: Option<crate::models::InteriorLight>) {
        self.entities[id].light = light;
    }

    pub fn update_entity_vertices(&self, gpu: &Gpu, id: usize, vertices: &[ModelVertex]) {
        gpu.queue.write_buffer(&self.entities[id].model.vertex_buf, 0, bytemuck::cast_slice(vertices));
    }

    pub fn set_sky(&mut self, gpu: &Gpu, assets: &crate::skygpu::SkyAssets) {
        self.sky = Some(crate::skygpu::SkyGpu::new(gpu, self.format, assets));
    }

    pub fn set_weather_assets(&mut self, gpu: &Gpu, assets: &crate::weathergpu::WeatherAssets) {
        self.weather = Some(crate::weathergpu::WeatherGpu::new(gpu, self.format, assets));
    }

    /// Takes the weather as it is now to be drawn by the next `render`.
    pub fn prepare_weather(&mut self, gpu: &Gpu, weather: &crate::weather::Weather, camera: &Camera, size: (u32, u32)) {
        if let Some(w) = &mut self.weather {
            w.prepare(gpu, weather, &camera.view_proj_relative(), camera.pos, size, &self.atmosphere);
        }
    }

    pub fn clear_color(&self) -> wgpu::Color {
        let f = self.atmosphere.fog_color;
        wgpu::Color { r: f.x as f64, g: f.y as f64, b: f.z as f64, a: 1.0 }
    }

    pub fn upload_tile(&mut self, gpu: &Gpu, tile: &TileCpu) -> TileGpu {
        let (device, queue) = (&gpu.device, &gpu.queue);

        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain vertices"),
            contents: bytemuck::cast_slice(&tile.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain indices"),
            contents: bytemuck::cast_slice(&tile.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let gv = create_texture_array(gpu, &tile.textures, "ground textures");

        let alpha = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("alpha maps"),
            size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 256 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &alpha,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &tile.alpha,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64 * 4), rows_per_image: Some(64) },
            wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 256 },
        );

        let array_view = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tile uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let av = array_view(&alpha);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile bind group"),
            layout: &self.tile_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&gv) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.ground_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&av) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.alpha_sampler) },
            ],
        });

        let water = (!tile.water.is_empty()).then(|| {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water vertices"),
                contents: bytemuck::cast_slice(&tile.water),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buf, tile.water.len() as u32)
        });

        // Group placements by model so each model is one instanced draw per batch.
        let mut by_model: HashMap<&str, (Arc<ModelCpu>, Vec<f32>, Vec<Mat4>)> = HashMap::new();
        for d in tile.doodads.iter().filter(|d| d.drawn) {
            let entry = by_model.entry(d.model.key.as_str()).or_insert_with(|| (d.model.clone(), Vec::new(), Vec::new()));
            entry.1.extend_from_slice(&instance_floats(&d.matrix, d.light));
            entry.2.push(d.matrix);
        }
        let mut doodads = Vec::with_capacity(by_model.len());
        let mut animated = Vec::new();
        for (_, (model_cpu, matrices, placed)) in by_model {
            let model = self.model_gpu(gpu, &model_cpu);
            let mut bounds = NO_BOUNDS;
            for m in &placed {
                grow_transformed(&mut bounds, m, model.bounds);
            }
            if let Some(tracks) = model_cpu.tracks.as_ref().filter(|t| t.animated) {
                animated.push((model.clone(), tracks.clone(), model_cpu.batches.iter().map(|b| b.refs).collect()));
            }
            let instances = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("doodad instances"),
                contents: bytemuck::cast_slice(&matrices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            doodads.push(DoodadGroup { model, instances, count: (matrices.len() / INSTANCE_FLOATS) as u32, bounds });
        }

        let wmos = tile
            .wmos
            .iter()
            .map(|w| {
                let cols = instance_floats(&w.matrix, None);
                let groups: Vec<_> = w
                    .wmo
                    .groups
                    .iter()
                    .map(|g| {
                        g.model.as_ref().map(|m| {
                            let model = self.model_gpu(gpu, m);
                            let mut bounds = NO_BOUNDS;
                            grow_transformed(&mut bounds, &w.matrix, model.bounds);
                            DoodadGroup {
                                model,
                                instances: gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("wmo group instance"),
                                    contents: bytemuck::cast_slice(&cols),
                                    usage: wgpu::BufferUsages::VERTEX,
                                }),
                                count: 1,
                                bounds,
                            }
                        })
                    })
                    .collect();
                let visible = vec![Cell::new(true); groups.len()];
                WmoGpu { wmo: w.wmo.clone(), matrix: w.matrix, lamps: crate::models::lamps_of(&w.wmo, &w.matrix), groups, visible }
            })
            .collect();

        let mut bounds = NO_BOUNDS;
        tile.vertices.iter().for_each(|v| grow(&mut bounds, Vec3::from(v.pos)));
        tile.water.iter().for_each(|v| grow(&mut bounds, Vec3::from(v.pos)));

        TileGpu {
            bounds,
            vertex_buf,
            index_buf,
            index_count: tile.indices.len() as u32,
            uniform,
            bind_group,
            water,
            doodads,
            animated,
            wmos,
            shadow: tile.shadow.clone(),
            coord: tile.coord,
            origin: tile_origin(tile.coord),
        }
    }

    fn make_stage(&self, gpu: &Gpu) -> (wgpu::Buffer, wgpu::BindGroup) {
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("stage uniform"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let ground = create_texture_array(gpu, &[], "stage ground");
        let alpha = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("stage alpha"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stage bind group"),
            layout: &self.tile_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&ground) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.ground_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&alpha) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.alpha_sampler) },
            ],
        });
        (uniform, bind_group)
    }

    fn upload_model_texture(&mut self, gpu: &Gpu, key: &str, tex: &RawTexture) {
        if self.model_textures.contains_key(key) {
            return;
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("model texture"),
            size: wgpu::Extent3d { width: tex.width, height: tex.height, depth_or_array_layers: 1 },
            mip_level_count: tex.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (mip, data) in tex.mips.iter().enumerate() {
            let (w, h) = ((tex.width >> mip).max(1), (tex.height >> mip).max(1));
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        self.model_textures.insert(key.to_string(), texture.create_view(&Default::default()));
    }

    fn model_gpu(&mut self, gpu: &Gpu, model: &ModelCpu) -> Arc<ModelGpu> {
        if let Some(m) = self.models.get(&model.key) {
            return m.clone();
        }
        for b in &model.batches {
            if let Some((key, tex)) = &b.texture {
                self.upload_model_texture(gpu, key, tex);
            }
        }
        let device = &gpu.device;
        let batches = model
            .batches
            .iter()
            .map(|b| {
                let view = b.texture.as_ref().and_then(|(k, _)| self.model_textures.get(k)).unwrap_or(&self.grey);
                let alpha_test = if b.blend == Blend::AlphaKey { 0.5f32 } else { 0.0 };
                let base = [alpha_test, if b.unlit { 1.0f32 } else { 0.0 }, if b.particle { 1.0 } else { 0.0 }, match b.blend { Blend::Modulate => 1.0, Blend::Modulate2x => 2.0, _ => 0.0 }];
                // What a batch looks like at the start of its tracks is what it looks like all through if they do not move.
                let first = model.tracks.as_ref().map_or(crate::anim::BatchLook::STILL, |t| t.look(b.refs, 0.0));
                let lit = [b.baked.code(), if b.window { 1.0 } else { 0.0 }, b.glow[0] as f32 / 255.0, b.glow[1] as f32 / 255.0, b.glow[2] as f32 / 255.0];
                let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("batch params"),
                    contents: bytemuck::cast_slice(&batch_params(base, lit, &first)),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
                let sampler = &self.model_samplers[usize::from(b.wrap)];
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("model batch"),
                    layout: &self.model_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                    ],
                });
                BatchGpu { first: b.index_start, count: b.index_count, bind_group, blend: b.blend, params, base, lit }
            })
            .collect();
        let mut bounds = NO_BOUNDS;
        model.vertices.iter().for_each(|v| grow(&mut bounds, Vec3::from(v.pos)));
        if model.vertices.is_empty() {
            bounds = (Vec3::ZERO, Vec3::ZERO);
        }
        let built = Arc::new(ModelGpu {
            bounds,
            vertex_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("model vertices"),
                contents: bytemuck::cast_slice(&model.vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }),
            index_buf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("model indices"),
                contents: bytemuck::cast_slice(&model.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            batches,
        });
        self.models.insert(model.key.clone(), built.clone());
        built
    }

    fn ensure_depth(&mut self, gpu: &Gpu, size: (u32, u32)) -> &wgpu::TextureView {
        if self.depth.as_ref().map(|d| d.1) != Some(size) {
            let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth = Some((tex.create_view(&Default::default()), size));
        }
        &self.depth.as_ref().unwrap().0
    }

    /// A transparent picture `size` pixels big that models can be drawn into; returns its id and the interface
    /// texture that shows it.
    pub fn add_offscreen(&mut self, gpu: &Gpu, size: (u32, u32)) -> (usize, usize) {
        let offscreen = self.make_offscreen(gpu, size);
        let texture = self.ui.add_view(gpu, &offscreen.view);
        self.offscreens.push(offscreen);
        (self.offscreens.len() - 1, texture)
    }

    /// Give an offscreen picture (from `add_offscreen`) another size; what was drawn in it is gone.
    pub fn resize_offscreen(&mut self, gpu: &Gpu, target: (usize, usize), size: (u32, u32)) {
        let offscreen = self.make_offscreen(gpu, size);
        self.ui.replace_view(gpu, target.1, &offscreen.view);
        self.offscreens[target.0] = offscreen;
    }

    fn make_offscreen(&self, gpu: &Gpu, size: (u32, u32)) -> Offscreen {
        let make = |format, usage| gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        Offscreen {
            view: make(self.format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default()),
            depth: make(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default()),
        }
    }

    /// Draw the entities `ids` as seen by `camera` into offscreen picture `target`, in a neutral light and without
    /// fog, on a transparent background. Call before `render` of the same frame.
    pub fn render_offscreen<'a>(&mut self, gpu: &Gpu, target: usize, camera: &Camera, ids: &[usize], tiles: impl Iterator<Item = (TileCoord, &'a TileGpu)>) {
        let view_proj = camera.view_proj_relative();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            sun_dir: Vec3::new(-0.35, -0.55, -0.75).normalize().extend(0.0).to_array(),
            sun_color: [0.75, 0.72, 0.66, 0.0],
            ambient: [0.6, 0.6, 0.64, 0.0],
            fog_color: [0.0, 0.0, 0.0, 1.0],
            fog_params: [1.0e6, 2.0e6, 0.0, 0.0],
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            sky_top: [0.0; 4],
            sky_mid: [0.0; 4],
            light_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            shadow: [0.0; 4],
        };
        gpu.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        let cam_tile = tile_of(camera.pos.x, camera.pos.z);
        let origin = tile_origin(cam_tile);
        let tiles: Vec<&TileGpu> = tiles.map(|(_, t)| t).filter(|t| t.coord == cam_tile).collect();
        let off = [(origin.0 - camera.pos.x as f64) as f32, -camera.pos.y, (origin.1 - camera.pos.z as f64) as f32, 0.0];
        for t in &tiles {
            gpu.queue.write_buffer(&t.uniform, 0, bytemuck::cast_slice(&off));
        }
        if tiles.is_empty() {
            if self.stage.is_none() {
                self.stage = Some(self.make_stage(gpu));
            }
            gpu.queue.write_buffer(&self.stage.as_ref().expect("made above").0, 0, bytemuck::cast_slice(&off));
        }
        for &i in ids {
            let e = &self.entities[i];
            let local = Vec3::new((e.pos.x as f64 - origin.0) as f32, e.pos.y, (e.pos.z as f64 - origin.1) as f32);
            gpu.queue.write_buffer(&e.instance, 0, bytemuck::cast_slice(&instance_floats(&(Mat4::from_translation(local) * e.orient), e.light)));
        }
        let Some(off) = self.offscreens.get(target) else { return };
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("offscreen") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("offscreen"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &off.view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &off.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            for (pipeline, blends) in [(0, &[Blend::Opaque, Blend::AlphaKey][..]), (1, &[Blend::Alpha]), (2, &[Blend::Additive]), (3, &[Blend::Modulate]), (4, &[Blend::Modulate2x])] {
                draw_entities(&mut pass, &self.globals_bg, &self.model_pipelines[pipeline], &tiles, cam_tile, self.stage.as_ref().map(|s| &s.1), &self.entities, Some(ids), blends);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
    }

    /// Draw one frame of `tiles` into `target`.
    pub fn render<'a>(
        &mut self,
        gpu: &Gpu,
        target: &wgpu::TextureView,
        size: (u32, u32),
        camera: &Camera,
        time: f32,
        tiles: impl Iterator<Item = (TileCoord, &'a TileGpu)>,
    ) {
        let a = &self.atmosphere;
        let view_proj = camera.view_proj_relative();
        // The sun casts shadows while it is up.
        let shadows = self.shadows && a.sun_dir.y < -0.1;
        let light = light_view_proj(camera.pos, a.sun_dir);
        let texel = 2.0 * SHADOW_RADIUS / SHADOW_SIZE as f32;
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            sun_dir: a.sun_dir.extend(0.0).to_array(),
            // The fourth number of the sun's colour is how much the windows of buildings glow now.
            sun_color: a.sun_color.extend(crate::sky::window_glow(a.hour)).to_array(),
            ambient: a.ambient.extend(0.0).to_array(),
            fog_color: a.fog_color.extend(1.0).to_array(),
            fog_params: [a.fog_start, a.fog_end, time, 0.0],
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            sky_top: a.sky_top.extend(0.0).to_array(),
            sky_mid: a.sky_mid.extend(0.0).to_array(),
            light_view_proj: light.to_cols_array_2d(),
            shadow: [if shadows { 1.0 } else { 0.0 }, 2.0 * texel, 1.0 / SHADOW_SIZE as f32, 0.2 / SHADOW_DEPTH],
        };
        gpu.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        gpu.queue.write_buffer(&self.shadow_buf, 0, bytemuck::bytes_of(&Globals { view_proj: light.to_cols_array_2d(), ..globals }));

        let tiles: Vec<&TileGpu> = tiles.map(|(_, t)| t).collect();
        // The placed models whose looks animate (a flickering glow, water that runs).
        let cam_tile = tile_of(camera.pos.x, camera.pos.z);
        for t in tiles.iter().filter(|t| (t.coord.0 - cam_tile.0).abs() <= DOODAD_TILE_RADIUS && (t.coord.1 - cam_tile.1).abs() <= DOODAD_TILE_RADIUS) {
            for (model, tracks, refs) in &t.animated {
                let looks: Vec<_> = refs.iter().map(|&r| tracks.look(r, time * 1000.0)).collect();
                model.set_looks(gpu, &looks);
            }
        }
        for t in &tiles {
            let off = [
                (t.origin.0 - camera.pos.x as f64) as f32,
                -camera.pos.y,
                (t.origin.1 - camera.pos.z as f64) as f32,
                0.0,
            ];
            gpu.queue.write_buffer(&t.uniform, 0, bytemuck::cast_slice(&off));
        }

        // Which building groups show from here.
        for t in &tiles {
            if (t.coord.0 - cam_tile.0).abs() > DOODAD_TILE_RADIUS || (t.coord.1 - cam_tile.1).abs() > DOODAD_TILE_RADIUS {
                continue;
            }
            let offset = Vec3::new((t.origin.0 - camera.pos.x as f64) as f32, -camera.pos.y, (t.origin.1 - camera.pos.z as f64) as f32);
            let local = Vec3::new((camera.pos.x as f64 - t.origin.0) as f32, camera.pos.y, (camera.pos.z as f64 - t.origin.1) as f32);
            for w in &t.wmos {
                // WMO model space (x east, y north, z up) <-> render space (x east, y up, z south).
                let r = w.matrix.inverse().transform_point3(local);
                let to_clip = view_proj * Mat4::from_translation(offset) * w.matrix * crate::wmovis::MODEL_TO_RENDER;
                let visible = crate::wmovis::visible_groups(&w.wmo, Vec3::new(r.x, -r.z, r.y), &to_clip);
                for (cell, v) in w.visible.iter().zip(visible) {
                    cell.set(v);
                }
            }
        }

        self.ease_room_lights(&tiles, time);

        // Entities are placed relative to the camera's tile, whose uniform they are drawn with.
        let origin = tile_origin(cam_tile);
        for e in self.entities.iter().filter(|e| e.visible) {
            let local = Vec3::new((e.pos.x as f64 - origin.0) as f32, e.pos.y, (e.pos.z as f64 - origin.1) as f32);
            gpu.queue.write_buffer(&e.instance, 0, bytemuck::cast_slice(&instance_floats(&(Mat4::from_translation(local) * e.orient), e.light)));
        }

        if let Some(sky) = &mut self.sky {
            sky.prepare(gpu, &view_proj, &self.atmosphere, time);
        }
        let clear = self.clear_color();
        self.ensure_depth(gpu, size);
        let depth = &self.depth.as_ref().unwrap().0;

        // Tiles the camera cannot see (terrain and water; doodads are tested by group) and tiles the sun's map does not reach.
        let mut seen = Frustum::new(&view_proj, camera.pos);
        seen.reach = self.prop_reach;
        let lit = Frustum::new(&light, camera.pos);
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        if shadows {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // Only the tiles around the camera cast: those are the ones whose models are drawn.
            let near: Vec<&TileGpu> = tiles.iter().copied().filter(|t| (t.coord.0 - cam_tile.0).abs() <= DOODAD_TILE_RADIUS && (t.coord.1 - cam_tile.1).abs() <= DOODAD_TILE_RADIUS).collect();
            pass.set_pipeline(&self.shadow_terrain);
            pass.set_bind_group(0, &self.shadow_bg, &[]);
            for t in near.iter().filter(|t| t.index_count > 0 && lit.sees(t.origin, t.bounds)) {
                pass.set_bind_group(1, &t.bind_group, &[]);
                pass.set_vertex_buffer(0, t.vertex_buf.slice(..));
                pass.set_index_buffer(t.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..t.index_count, 0, 0..1);
            }
            // shortcut: translucent models cast nothing; upgrade if their missing shadows show.
            let casts = [Blend::Opaque, Blend::AlphaKey];
            draw_doodads(&mut pass, &self.shadow_bg, &self.shadow_model, &near, cam_tile, &casts, true, &lit);
            draw_entities(&mut pass, &self.shadow_bg, &self.shadow_model, &tiles, cam_tile, None, &self.entities, None, &casts);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(sky) = &self.sky {
                sky.draw(&mut pass);
            } else {
                pass.set_pipeline(&self.sky_pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.draw(0..3, 0..1);
            }

            pass.set_pipeline(&self.terrain_pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            for t in tiles.iter().filter(|t| t.index_count > 0 && seen.sees(t.origin, t.bounds)) {
                pass.set_bind_group(1, &t.bind_group, &[]);
                pass.set_vertex_buffer(0, t.vertex_buf.slice(..));
                pass.set_index_buffer(t.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..t.index_count, 0, 0..1);
            }

            draw_doodads(&mut pass, &self.globals_bg, &self.model_pipelines[0], &tiles, cam_tile, &[Blend::Opaque, Blend::AlphaKey], false, &seen);
            draw_entities(&mut pass, &self.globals_bg, &self.model_pipelines[0], &tiles, cam_tile, None, &self.entities, None, &[Blend::Opaque, Blend::AlphaKey]);

            // Translucent liquids after all opaque geometry.
            if let Some(water_bg) = &self.water_bg {
                pass.set_pipeline(&self.water_pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_bind_group(2, water_bg, &[]);
                for t in tiles.iter().filter(|t| seen.sees(t.origin, t.bounds)) {
                    if let Some((buf, count)) = &t.water {
                        pass.set_bind_group(1, &t.bind_group, &[]);
                        pass.set_vertex_buffer(0, buf.slice(..));
                        pass.draw(0..*count, 0..1);
                    }
                }
            }

            draw_doodads(&mut pass, &self.globals_bg, &self.model_pipelines[1], &tiles, cam_tile, &[Blend::Alpha], false, &seen);
            draw_entities(&mut pass, &self.globals_bg, &self.model_pipelines[1], &tiles, cam_tile, None, &self.entities, None, &[Blend::Alpha]);
            draw_doodads(&mut pass, &self.globals_bg, &self.model_pipelines[2], &tiles, cam_tile, &[Blend::Additive], false, &seen);
            draw_entities(&mut pass, &self.globals_bg, &self.model_pipelines[2], &tiles, cam_tile, None, &self.entities, None, &[Blend::Additive]);
            draw_doodads(&mut pass, &self.globals_bg, &self.model_pipelines[3], &tiles, cam_tile, &[Blend::Modulate], false, &seen);
            draw_entities(&mut pass, &self.globals_bg, &self.model_pipelines[3], &tiles, cam_tile, None, &self.entities, None, &[Blend::Modulate]);
            draw_doodads(&mut pass, &self.globals_bg, &self.model_pipelines[4], &tiles, cam_tile, &[Blend::Modulate2x], false, &seen);
            draw_entities(&mut pass, &self.globals_bg, &self.model_pipelines[4], &tiles, cam_tile, None, &self.entities, None, &[Blend::Modulate2x]);
            if let Some(weather) = &self.weather {
                weather.draw(&mut pass);
            }
        }
        // The gamma: brighter adds light over the whole picture, darker lays black over it.
        let (w, h, g) = (size.0 as f32, size.1 as f32, self.gamma);
        if g > 0.0 {
            self.ui.quad_first(Quad { texture: crate::ui::WHITE, rect: [0.0, 0.0, w, h], uv: FULL_UV, color: [g * 0.6, g * 0.6, g * 0.6, 1.0], additive: true });
        } else if g < 0.0 {
            self.ui.quad_first(Quad { texture: crate::ui::WHITE, rect: [0.0, 0.0, w, h], uv: FULL_UV, color: [0.0, 0.0, 0.0, -g * 0.6], additive: false });
        }
        self.ui.draw(gpu, &mut encoder, target, size);
        gpu.queue.submit(Some(encoder.finish()));
    }
}

/// Draw the visible entities with the uniform of the tile they are placed in.
#[allow(clippy::too_many_arguments)]
fn draw_entities(
    pass: &mut wgpu::RenderPass<'_>,
    globals_bg: &wgpu::BindGroup,
    pipeline: &wgpu::RenderPipeline,
    tiles: &[&TileGpu],
    cam_tile: TileCoord,
    // What to stand on when the camera is over no tile.
    stage: Option<&wgpu::BindGroup>,
    entities: &[EntityGpu],
    only: Option<&[usize]>,
    blends: &[Blend],
) {
    let Some(ground) = tiles.iter().find(|t| t.coord == cam_tile).map(|t| &t.bind_group).or(stage) else { return };
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, globals_bg, &[]);
    pass.set_bind_group(1, ground, &[]);
    for (_, e) in entities.iter().enumerate().filter(|(i, e)| e.visible && only.is_none_or(|o| o.contains(i))) {
        pass.set_vertex_buffer(0, e.model.vertex_buf.slice(..));
        pass.set_vertex_buffer(1, e.instance.slice(..));
        pass.set_index_buffer(e.model.index_buf.slice(..), wgpu::IndexFormat::Uint16);
        for b in e.model.batches.iter().filter(|b| blends.contains(&if e.fade < 1.0 && matches!(b.blend, Blend::Opaque | Blend::AlphaKey) { Blend::Alpha } else { b.blend })) {
            pass.set_bind_group(2, &b.bind_group, &[]);
            pass.draw_indexed(b.first..b.first + b.count, 0, 0..1);
        }
    }
}

/// Instanced draw of every placed model whose batches use one of `blends`.
fn draw_doodads(
    pass: &mut wgpu::RenderPass<'_>,
    globals_bg: &wgpu::BindGroup,
    pipeline: &wgpu::RenderPipeline,
    tiles: &[&TileGpu],
    cam_tile: TileCoord,
    blends: &[Blend],
    // Building groups the camera does not see too (shadows must not pop in and out with the view).
    unseen: bool,
    // Groups whose placements are all outside this are skipped.
    view: &Frustum,
) {
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, globals_bg, &[]);
    for t in tiles {
        if (t.coord.0 - cam_tile.0).abs() > DOODAD_TILE_RADIUS || (t.coord.1 - cam_tile.1).abs() > DOODAD_TILE_RADIUS {
            continue;
        }
        pass.set_bind_group(1, &t.bind_group, &[]);
        let wmo_groups = t.wmos.iter().flat_map(|w| w.groups.iter().zip(&w.visible).filter(|(_, v)| unseen || v.get()).filter_map(|(g, _)| g.as_ref()));
        // Buildings are always drawn; the props (trees, fences) only as far as the environment detail says.
        let props = t.doodads.iter().filter(|g| view.near(t.origin, g.bounds));
        for g in props.chain(wmo_groups).filter(|g| g.model.has(blends) && view.sees(t.origin, g.bounds)) {
            pass.set_vertex_buffer(0, g.model.vertex_buf.slice(..));
            pass.set_vertex_buffer(1, g.instances.slice(..));
            pass.set_index_buffer(g.model.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            for b in g.model.batches.iter().filter(|b| blends.contains(&b.blend)) {
                pass.set_bind_group(2, &b.bind_group, &[]);
                pass.draw_indexed(b.first..b.first + b.count, 0, 0..g.count);
            }
        }
    }
}
