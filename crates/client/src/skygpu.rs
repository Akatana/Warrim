//! Draws the sky of `heavens.rs`: the dome, the stars, the sun and the moon, the clouds. Everything is a direction from the camera at the far
//! depth, drawn first in the frame; the world is drawn over it.

use crate::anim::{BatchLook, BatchRefs, LookTracks};
use crate::clouds::Clouds;
use crate::heavens::{self, Vertex};
use crate::models::{ModelCache, ModelCpu, RawTexture};
use crate::renderer::{Atmosphere, Gpu, DEPTH_FORMAT};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use mpq::Mount;
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

impl From<Vertex> for SkyVertex {
    fn from(v: Vertex) -> Self {
        SkyVertex { pos: v.pos.to_array(), uv: v.uv, color: v.color }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
}

/// The pictures of the sky, from the game's files.
pub struct SkyAssets {
    sun_glare: Arc<RawTexture>,
    moon_glare: Arc<RawTexture>,
    sun_center: Arc<RawTexture>,
    moon: Arc<RawTexture>,
    stars: Arc<ModelCpu>,
}

impl SkyAssets {
    pub fn load(mount: &Mount) -> Result<Self> {
        let cache = ModelCache::default();
        let texture = |name: &str| cache.texture(mount, name).map(|(_, t)| t).with_context(|| format!("no sky texture {name}"));
        Ok(SkyAssets {
            sun_glare: texture(r"Textures\sunGlare.blp")?,
            moon_glare: texture(r"Textures\moonGlare.blp")?,
            sun_center: texture(r"Textures\sunCenter.blp")?,
            moon: texture(r"Textures\moon.blp")?,
            stars: cache.plain(mount, r"Environments\Stars\Stars.m2")?,
        })
    }
}

pub fn texture_view(gpu: &Gpu, tex: &RawTexture) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky texture"),
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
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: mip as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
    texture.create_view(&Default::default())
}

fn buffer<T: Pod>(gpu: &Gpu, label: &str, data: &[T], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::cast_slice(data), usage })
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

impl Mesh {
    fn new(gpu: &Gpu, label: &str, vertices: &[SkyVertex], indices: &[u16]) -> Self {
        Mesh {
            vertices: buffer(gpu, label, vertices, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
            indices: buffer(gpu, label, indices, wgpu::BufferUsages::INDEX),
            count: indices.len() as u32,
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.count, 0, 0..1);
    }
}

/// A picture in a bind group of its own.
struct Picture {
    bind_group: wgpu::BindGroup,
}

pub struct SkyGpu {
    flat: wgpu::RenderPipeline,
    cloud: wgpu::RenderPipeline,
    alpha: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    dome: Mesh,
    dome_points: Vec<Vec3>,
    clouds: Mesh,
    /// The picture of the clouds, made on the CPU (`clouds.rs`) and sent when a band of it has changed.
    cloud_state: Clouds,
    cloud_texture: wgpu::Texture,
    cloud_picture: Picture,
    cloud_filled: bool,
    last_time: Option<f32>,
    /// Sun's glare, moon's glare, sun, moon: a square each.
    discs: Mesh,
    glare: Picture,
    moon_glare: Picture,
    sun: Picture,
    moon: Picture,
    /// The star model's triangles as vertices, batch by batch: where and which part of its picture.
    stars: wgpu::Buffer,
    star_corners: Vec<(Vec3, [f32; 2])>,
    star_batches: Vec<StarBatch>,
    star_tracks: Option<Arc<LookTracks>>,
}

/// A part of the star model that has a picture of its own, its place among the corners, and how its colour, opacity and picture move.
struct StarBatch {
    first: u32,
    count: u32,
    refs: BatchRefs,
    picture: Picture,
}

/// The way the stars turn once a day: round a pole that stands 30 degrees above the horizon in the north.
fn star_turn(hour: f32) -> Quat {
    let pole = heavens::to_render(Vec3::new(30f32.to_radians().cos(), 0.0, 30f32.to_radians().sin()));
    Quat::from_axis_angle(pole, hour / 24.0 * std::f32::consts::TAU)
}

impl SkyGpu {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, assets: &SkyAssets) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky dome"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/skydome.wgsl").into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let picture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky picture layout"),
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
        // The stars' pictures scroll and turn (their uv is transformed each frame), so they must wrap: clamping smears the edge texels into streaks.
        let wrapping = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let picture_with = |tex: &RawTexture, sampler: &wgpu::Sampler| {
            let view = texture_view(gpu, tex);
            Picture {
                bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky picture"),
                    layout: &picture_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) }],
                }),
            }
        };
        let picture = |tex: &RawTexture| picture_with(tex, &sampler);

        let layout = |groups: &[&wgpu::BindGroupLayout]| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky pipeline layout"), bind_group_layouts: groups, push_constant_ranges: &[] });
        let untextured = layout(&[&uniform_layout]);
        let textured = layout(&[&uniform_layout, &picture_layout]);
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let pipeline = |label: &str, layout: &wgpu::PipelineLayout, fragment: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<SkyVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: false, depth_compare: wgpu::CompareFunction::Always, stencil: Default::default(), bias: Default::default() }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let additive_blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::OVER,
        };

        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky uniform"), size: std::mem::size_of::<Uniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky uniform"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });

        let (dome_points, dome_indices) = heavens::dome();
        let blank = SkyVertex { pos: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] };
        let (cloud_vertices, cloud_indices) = heavens::cloud_dome();
        let cloud_vertices: Vec<SkyVertex> = cloud_vertices.into_iter().map(Into::into).collect();
        let quad = |n: u16| [0, 2, 1, 1, 2, 3].map(|i| i + 4 * n);
        let disc_indices: Vec<u16> = (0..4).flat_map(quad).collect();

        // The stars: every batch of the model gets its own corners, so that its picture can move without moving a neighbour's.
        let stars = &assets.stars;
        let mut star_corners = Vec::new();
        let mut star_batches = Vec::new();
        for b in &stars.batches {
            let Some((_, texture)) = &b.texture else { continue };
            let first = star_corners.len() as u32;
            let indices = &stars.indices[b.index_start as usize..(b.index_start + b.index_count) as usize];
            star_corners.extend(indices.iter().map(|&i| (Vec3::from(stars.vertices[i as usize].pos), stars.vertices[i as usize].uv)));
            star_batches.push(StarBatch { first, count: b.index_count, refs: b.refs, picture: picture_with(texture, &wrapping) });
        }

        // The clouds: the original's `SkyCloudLOD` is 1 (a picture of 256 texels) unless set otherwise.
        let cloud_state = Clouds::new(1, 0x5eed_c10d);
        let side = cloud_state.size() as u32;
        let cloud_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("clouds"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let cloud_view = cloud_texture.create_view(&Default::default());
        let cloud_picture = Picture {
            bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("clouds"),
                layout: &picture_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&cloud_view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
            }),
        };

        SkyGpu {
            flat: pipeline("sky dome", &untextured, "fs_flat", None),
            cloud: pipeline("sky clouds", &textured, "fs_cloud", Some(wgpu::BlendState::ALPHA_BLENDING)),
            alpha: pipeline("sky sprites", &textured, "fs_tex", Some(wgpu::BlendState::ALPHA_BLENDING)),
            additive: pipeline("sky glare", &textured, "fs_tex", Some(additive_blend)),
            uniform,
            uniform_bg,
            dome: Mesh::new(gpu, "sky dome", &vec![blank; dome_points.len()], &dome_indices),
            dome_points,
            clouds: Mesh::new(gpu, "sky clouds", &cloud_vertices, &cloud_indices),
            cloud_state,
            cloud_texture,
            cloud_picture,
            cloud_filled: false,
            last_time: None,
            discs: Mesh::new(gpu, "sky discs", &[blank; 16], &disc_indices),
            glare: picture(&assets.sun_glare),
            moon_glare: picture(&assets.moon_glare),
            sun: picture(&assets.sun_center),
            moon: picture(&assets.moon),
            stars: buffer(gpu, "sky stars", &vec![blank; star_corners.len()], wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
            star_corners,
            star_batches,
            star_tracks: stars.tracks.clone(),
        }
    }

    /// Writes what this frame's sky looks like. `time` is seconds, for the wind in the clouds.
    pub fn prepare(&mut self, gpu: &Gpu, view_proj: &Mat4, a: &Atmosphere, time: f32) {
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&Uniform { view_proj: view_proj.to_cols_array_2d() }));

        // The clouds: the whole picture the first time, then a band of rows every tenth of a second, faded between whole pictures.
        let dt = self.last_time.map_or(0.0, |t| (time - t).clamp(0.0, 0.5));
        self.last_time = Some(time);
        if !self.cloud_filled {
            self.cloud_filled = true;
            self.cloud_state.fill(a);
        } else {
            self.cloud_state.update(dt, a);
        }
        let side = self.cloud_state.size() as u32;
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.cloud_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            self.cloud_state.blended(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 4), rows_per_image: Some(side) },
            wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
        );

        let dome: Vec<SkyVertex> = self
            .dome_points
            .iter()
            .zip(heavens::dome_colors(a))
            .map(|(&p, c)| SkyVertex { pos: heavens::to_render(p).to_array(), uv: [0.0; 2], color: [c.x, c.y, c.z, 1.0] })
            .collect();
        gpu.queue.write_buffer(&self.dome.vertices, 0, bytemuck::cast_slice(&dome));

        // The weather hides what is above the clouds.
        let clear = 1.0 - a.storm;
        let discs: Vec<SkyVertex> = [
            heavens::billboard(heavens::sun_direction(a.hour), GLARE_SIZE, a.celestial, heavens::sun_glare(a.hour) * clear),
            heavens::billboard(heavens::moon_direction(a.hour), GLARE_SIZE * 0.75, a.celestial, heavens::moon_glare(a.hour) * clear),
            heavens::billboard(heavens::sun_direction(a.hour), heavens::sun_size(a.hour), a.celestial, clear),
            heavens::billboard(heavens::moon_direction(a.hour), heavens::moon_size(a.hour), a.celestial, clear),
        ]
        .into_iter()
        .flatten()
        .map(Into::into)
        .collect();
        gpu.queue.write_buffer(&self.discs.vertices, 0, bytemuck::cast_slice(&discs));

        let turn = star_turn(a.hour);
        let light = heavens::star_light(a.hour) * clear;
        let mut stars = Vec::with_capacity(self.star_corners.len());
        for b in &self.star_batches {
            let look = self.star_tracks.as_ref().map_or(BatchLook::STILL, |t| t.look(b.refs, time * 1000.0));
            let [[a, b1, c], [d, e, f]] = look.uv;
            let color = [look.tint[0], look.tint[1], look.tint[2], light * look.opacity];
            stars.extend(self.star_corners[b.first as usize..(b.first + b.count) as usize].iter().map(|&(p, uv)| SkyVertex {
                pos: (turn * p).to_array(),
                uv: [a * uv[0] + b1 * uv[1] + c, d * uv[0] + e * uv[1] + f],
                color,
            }));
        }
        gpu.queue.write_buffer(&self.stars, 0, bytemuck::cast_slice(&stars));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.uniform_bg, &[]);
        pass.set_pipeline(&self.flat);
        self.dome.draw(pass);

        // Stars, then the glare, the sun and the moon, then the clouds over all of them.
        pass.set_pipeline(&self.alpha);
        pass.set_vertex_buffer(0, self.stars.slice(..));
        for b in &self.star_batches {
            pass.set_bind_group(1, &b.picture.bind_group, &[]);
            pass.draw(b.first..b.first + b.count, 0..1);
        }
        pass.set_vertex_buffer(0, self.discs.vertices.slice(..));
        pass.set_index_buffer(self.discs.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.set_pipeline(&self.additive);
        pass.set_bind_group(1, &self.glare.bind_group, &[]);
        pass.draw_indexed(0..6, 0, 0..1);
        pass.set_pipeline(&self.alpha);
        pass.set_bind_group(1, &self.moon_glare.bind_group, &[]);
        pass.draw_indexed(6..12, 0, 0..1);
        pass.set_bind_group(1, &self.sun.bind_group, &[]);
        pass.draw_indexed(12..18, 0, 0..1);
        pass.set_bind_group(1, &self.moon.bind_group, &[]);
        pass.draw_indexed(18..24, 0, 0..1);

        pass.set_pipeline(&self.cloud);
        pass.set_bind_group(1, &self.cloud_picture.bind_group, &[]);
        self.clouds.draw(pass);
    }
}

/// The side of the glare round the sun, in the units of `heavens::RADIUS`.
// shortcut: the original scales the glare with how squarely the camera faces the sun (FUN_006cf490) and its sizes are fields of the light
// objects (4.0 and 1.5238 for the sun, 3.03 for the moon) whose use I did not follow; a fixed square and additive blending look right by eye.
const GLARE_SIZE: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sky_pictures_are_in_the_game_files() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let assets = SkyAssets::load(&mount).expect("sky pictures load");
        // The star model has batches with pictures, and its vertices are around the camera.
        assert!(assets.stars.batches.iter().any(|b| b.texture.is_some()));
        assert!(assets.stars.vertices.iter().all(|v| Vec3::from(v.pos).length() < 40.0));
    }
}
