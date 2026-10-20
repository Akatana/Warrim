// Depth-only pass from the sun: terrain and models (the view_proj of the globals is the sun's).

struct Globals {
    view_proj: mat4x4<f32>,
};

struct TileUniform {
    offset: vec4<f32>,
};

struct Params {
    v: vec4<f32>,
    tint: vec4<f32>,
    uv0: vec4<f32>,
    uv1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<uniform> tile: TileUniform;
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;
@group(2) @binding(2) var<uniform> params: Params;

@vertex
fn vs_terrain(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return g.view_proj * vec4<f32>(pos + tile.offset.xyz, 1.0);
}

struct ModelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_model(
    @location(0) pos: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
) -> ModelOut {
    let model = mat4x4<f32>(m0, m1, m2, m3);
    var out: ModelOut;
    out.clip = g.view_proj * vec4<f32>((model * vec4<f32>(pos, 1.0)).xyz + tile.offset.xyz, 1.0);
    out.uv = uv;
    return out;
}

// Leaves and fences cast the shadow of their picture, not of their quad.
@fragment
fn fs_model(in: ModelOut) {
    let uv = vec2<f32>(dot(params.uv0.xyz, vec3<f32>(in.uv, 1.0)), dot(params.uv1.xyz, vec3<f32>(in.uv, 1.0)));
    if (textureSample(tex, samp, uv).a < params.v.x) {
        discard;
    }
}
