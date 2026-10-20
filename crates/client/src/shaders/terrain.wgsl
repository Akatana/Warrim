// Terrain: up to four ground layers blended by per-chunk 64x64 alpha maps.

struct TileUniform {
    // tile corner minus camera position, computed in f64 on the CPU
    offset: vec4<f32>,
};

@group(1) @binding(0) var<uniform> tile: TileUniform;
@group(1) @binding(1) var ground: texture_2d_array<f32>;
@group(1) @binding(2) var ground_samp: sampler;
@group(1) @binding(3) var alpha_tex: texture_2d_array<f32>;
@group(1) @binding(4) var alpha_samp: sampler;

const NO_LAYER: u32 = 0xFFFFFFFFu;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) alpha_uv: vec2<f32>,
    @location(4) layers: vec4<u32>,
    @location(5) chunk: u32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) alpha_uv: vec2<f32>,
    @location(4) @interpolate(flat) layers: vec4<u32>,
    @location(5) @interpolate(flat) chunk: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let rel = in.pos + tile.offset.xyz;
    out.clip = g.view_proj * vec4<f32>(rel, 1.0);
    out.rel = rel;
    out.normal = in.normal;
    out.uv = in.uv;
    out.alpha_uv = in.alpha_uv;
    out.layers = in.layers;
    out.chunk = in.chunk;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Sample every layer unconditionally: textureSample needs uniform control flow.
    let last = textureNumLayers(ground) - 1u;
    let i0 = min(in.layers.x, last);
    let i1 = min(in.layers.y, last);
    let i2 = min(in.layers.z, last);
    let i3 = min(in.layers.w, last);

    let a = textureSample(alpha_tex, alpha_samp, in.alpha_uv, in.chunk);
    var col = textureSample(ground, ground_samp, in.uv, i0).rgb;
    let c1 = textureSample(ground, ground_samp, in.uv, i1).rgb;
    let c2 = textureSample(ground, ground_samp, in.uv, i2).rgb;
    let c3 = textureSample(ground, ground_samp, in.uv, i3).rgb;
    col = mix(col, c1, select(0.0, a.r, in.layers.y != NO_LAYER));
    col = mix(col, c2, select(0.0, a.g, in.layers.z != NO_LAYER));
    col = mix(col, c3, select(0.0, a.b, in.layers.w != NO_LAYER));

    let n = normalize(in.normal);
    let ndl = max(dot(n, -g.sun_dir.xyz), 0.0);
    let lit = col * min(g.ambient.rgb + g.sun_color.rgb * ndl * shadow_at(in.rel, n), vec3<f32>(1.3));

    let dist = length(in.rel);
    let f = clamp((dist - g.fog_params.x) / (g.fog_params.y - g.fog_params.x), 0.0, 1.0);
    return vec4<f32>(mix(lit, g.fog_color.rgb, f), 1.0);
}
