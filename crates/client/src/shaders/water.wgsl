// Animated liquids: 4 kinds x 30 frames in one texture array, alpha blended.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>, // x = fog start, y = fog end, z = time in seconds
};

struct TileUniform {
    offset: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<uniform> tile: TileUniform;
@group(2) @binding(0) var water_tex: texture_2d_array<f32>;
@group(2) @binding(1) var water_samp: sampler;

const FRAMES: u32 = 30u;
const FPS: f32 = 14.0;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) kind: u32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let rel = in.pos + tile.offset.xyz;
    out.clip = g.view_proj * vec4<f32>(rel, 1.0);
    out.rel = rel;
    out.uv = in.uv;
    out.kind = in.kind;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frame = u32(g.fog_params.z * FPS) % FRAMES;
    let tex = textureSample(water_tex, water_samp, in.uv, in.kind * FRAMES + frame);
    let light = min(g.ambient.rgb + g.sun_color.rgb * 0.6, vec3<f32>(1.2));

    // Kinds 0/1 (water, ocean): the texture is black RGB whose alpha channel holds the
    // ripple pattern, so build the colour from a tint plus ripple highlights.
    // Kinds 2/3 (magma, slime): ordinary opaque colour textures.
    let tint = select(vec3<f32>(0.18, 0.40, 0.46), vec3<f32>(0.09, 0.24, 0.40), in.kind == 1u);
    let water_color = (tint + vec3<f32>(tex.a * 0.55)) * light;
    let water_opacity = clamp(0.50 + tex.a * 0.6, 0.0, 0.88);

    let is_liquid_rgb = in.kind >= 2u;
    let lit = select(water_color, tex.rgb * light, is_liquid_rgb);
    let opacity = select(water_opacity, 1.0, is_liquid_rgb);

    let dist = length(in.rel);
    let f = clamp((dist - g.fog_params.x) / (g.fog_params.y - g.fog_params.x), 0.0, 1.0);
    return vec4<f32>(mix(lit, g.fog_color.rgb, f), opacity);
}
