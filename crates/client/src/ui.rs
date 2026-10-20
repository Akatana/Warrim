//! 2D interface drawing: textured, tinted quads in screen pixels, drawn over the finished frame with
//! alpha or additive blending (the original's `alphaMode="ADD"` highlights).

use crate::renderer::Gpu;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    /// Where in the quad this corner is (-1 to 1 each way) and whether the quad is round (a portrait): outside the circle
    /// that touches the sides nothing is drawn.
    shape: [f32; 3],
}

/// One rectangle: `rect` is (x, y, width, height) in pixels from the top left, `uv` is (u0, v0, u1, v1).
#[derive(Clone, Copy)]
pub struct Quad {
    pub texture: usize,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub additive: bool,
}

pub const FULL_UV: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
/// A 1x1 white texture, always texture 0: tint it for solid rectangles.
pub const WHITE: usize = 0;

pub struct Ui {
    pipelines: [wgpu::RenderPipeline; 2],
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Vec<wgpu::BindGroup>,
    quads: Vec<Quad>,
    /// Quads queued while this is set are cut to it (x, y, width, height in pixels): the inside of a scroll frame.
    clip: Option<[f32; 4]>,
    /// Quads queued while this is set are round; `rounds` says it for each queued quad.
    round: bool,
    rounds: Vec<bool>,
    /// Quads queued while this is set are turned (clockwise on the screen) by an angle in radians about a point in pixels;
    /// `turns` says it for each queued quad.
    turn: Option<(f32, f32, f32)>,
    turns: Vec<Option<(f32, f32, f32)>>,
    vertex_buf: wgpu::Buffer,
    capacity: usize,
}

impl Ui {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Ui {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ui"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ui.wgsl").into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui texture"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x3];
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::OVER,
        };
        let make = |label: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let capacity = 1024;
        let mut ui = Ui {
            pipelines: [make("ui alpha", wgpu::BlendState::ALPHA_BLENDING), make("ui additive", additive)],
            layout,
            sampler,
            textures: Vec::new(),
            quads: Vec::new(), clip: None, round: false, rounds: Vec::new(), turn: None, turns: Vec::new(),
            vertex_buf: Self::buffer(device, capacity),
            capacity,
        };
        assert_eq!(ui.add_texture(gpu, 1, 1, &[255; 4]), WHITE);
        ui
    }

    fn buffer(device: &wgpu::Device, quads: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui vertices"),
            size: (quads * 6 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload an RGBA8 image; the returned id goes into `Quad::texture`.
    pub fn add_texture(&mut self, gpu: &Gpu, width: u32, height: u32, rgba: &[u8]) -> usize {
        let texture = gpu.device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("ui texture"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        );
        let view = texture.create_view(&Default::default());
        self.textures.push(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui texture"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
        }));
        self.textures.len() - 1
    }

    fn view_group(&self, gpu: &Gpu, view: &wgpu::TextureView) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui view"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
        })
    }

    /// Show an existing texture (an offscreen render target) as an interface texture.
    pub fn add_view(&mut self, gpu: &Gpu, view: &wgpu::TextureView) -> usize {
        let group = self.view_group(gpu, view);
        self.textures.push(group);
        self.textures.len() - 1
    }

    /// Show another texture in the place of an interface texture made by `add_view`.
    pub fn replace_view(&mut self, gpu: &Gpu, texture: usize, view: &wgpu::TextureView) {
        self.textures[texture] = self.view_group(gpu, view);
    }

    /// Queue a quad for the next `draw`; quads are drawn in the order queued.
    pub fn quad(&mut self, mut q: Quad) {
        if let Some([cx, cy, cw, ch]) = self.clip {
            let [x, y, w, h] = q.rect;
            let (x0, y0, x1, y1) = (x.max(cx), y.max(cy), (x + w).min(cx + cw), (y + h).min(cy + ch));
            if w <= 0.0 || h <= 0.0 || x1 <= x0 || y1 <= y0 {
                return;
            }
            // The part of the picture that is left is the matching part of its coordinates.
            let [u0, v0, u1, v1] = q.uv;
            let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
            q.uv = [lerp(u0, u1, (x0 - x) / w), lerp(v0, v1, (y0 - y) / h), lerp(u0, u1, (x1 - x) / w), lerp(v0, v1, (y1 - y) / h)];
            q.rect = [x0, y0, x1 - x0, y1 - y0];
        }
        self.quads.push(q);
        self.rounds.push(self.round);
        self.turns.push(self.turn);
    }

    /// Queue a quad to be drawn before all the others (the screen's gamma goes under the interface).
    pub fn quad_first(&mut self, q: Quad) {
        self.quads.insert(0, q);
        self.rounds.insert(0, false);
        self.turns.insert(0, None);
    }

    /// Turn the quads queued from now on by `angle` radians (clockwise) about the point `(x, y)` in pixels, or stop doing so.
    pub fn set_turn(&mut self, turn: Option<(f32, f32, f32)>) {
        self.turn = turn;
    }

    /// Make the quads queued from now on round (or square again).
    pub fn set_round(&mut self, round: bool) {
        self.round = round;
    }

    /// Cut the quads queued from now on to a rectangle (pixels), or stop doing so.
    pub fn set_clip(&mut self, clip: Option<[f32; 4]>) {
        self.clip = clip;
    }

    /// Drop the queued quads without drawing them (a frame that is not shown).
    pub fn discard(&mut self) {
        self.quads.clear();
        self.rounds.clear();
        self.turns.clear();
    }

    /// Draw and clear the queued quads over `target`.
    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32)) {
        if self.quads.is_empty() {
            return;
        }
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let mut vertices: Vec<Vertex> = Vec::with_capacity(self.quads.len() * 6);
        for ((q, &round), turn) in self.quads.iter().zip(&self.rounds).zip(&self.turns) {
            let [x, y, qw, qh] = q.rect;
            let [u0, v0, u1, v1] = q.uv;
            let r = round as u32 as f32;
            let p = |px: f32, py: f32, u: f32, v: f32, lx: f32, ly: f32| {
                // A turned quad has its corners turned about the pivot.
                let (px, py) = turn.map_or((px, py), |(a, cx, cy)| {
                    let (sin, cos) = a.sin_cos();
                    (cx + (px - cx) * cos - (py - cy) * sin, cy + (px - cx) * sin + (py - cy) * cos)
                });
                Vertex { pos: [px / w * 2.0 - 1.0, 1.0 - py / h * 2.0], uv: [u, v], color: q.color, shape: [lx, ly, r] }
            };
            let (a, b, c, d) = (p(x, y, u0, v0, -1.0, -1.0), p(x + qw, y, u1, v0, 1.0, -1.0), p(x + qw, y + qh, u1, v1, 1.0, 1.0), p(x, y + qh, u0, v1, -1.0, 1.0));
            vertices.extend_from_slice(&[a, b, c, a, c, d]);
        }
        if self.quads.len() > self.capacity {
            self.capacity = self.quads.len().next_power_of_two();
            self.vertex_buf = Self::buffer(&gpu.device, self.capacity);
        }
        gpu.queue.write_buffer(&self.vertex_buf, 0, bytemuck::cast_slice(&vertices));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
        // One draw per run of quads with the same texture and blending.
        let mut start = 0;
        while start < self.quads.len() {
            let (texture, additive) = (self.quads[start].texture, self.quads[start].additive);
            let end = (start..self.quads.len()).find(|&i| self.quads[i].texture != texture || self.quads[i].additive != additive).unwrap_or(self.quads.len());
            pass.set_pipeline(&self.pipelines[additive as usize]);
            pass.set_bind_group(0, &self.textures[texture], &[]);
            pass.draw(start as u32 * 6..end as u32 * 6, 0..1);
            start = end;
        }
        drop(pass);
        self.quads.clear();
        self.rounds.clear();
        self.turns.clear();
    }
}
