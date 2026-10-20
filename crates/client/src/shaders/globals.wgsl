// Shared by the terrain and model shaders (prepended): the frame's globals and the sun's shadow map.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>, // x = start, y = end
    inv_view_proj: mat4x4<f32>,
    sky_top: vec4<f32>,
    sky_mid: vec4<f32>,
    light_view_proj: mat4x4<f32>,
    // x = 1 when shadows are on, y = how far a point moves along its normal before the lookup, z = one map texel in uv, w = depth bias
    shadow: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_samp: sampler_comparison;

/// How much of the sun reaches the camera-relative point `rel` whose surface faces `n`: 1 in the light, 0 in the shade.
fn shadow_at(rel: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.shadow.x < 0.5) {
        return 1.0;
    }
    let p = g.light_view_proj * vec4<f32>(rel + n * g.shadow.y, 1.0);
    let edge = max(abs(p.x), abs(p.y));
    if (edge >= 1.0 || p.z > 1.0) {
        return 1.0;
    }
    let uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    var lit = 0.0;
    // 4x4 taps, each already filtered 2x2: a soft edge, so the sun creeping over the map's texels does not show.
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            lit += textureSampleCompareLevel(shadow_map, shadow_samp, uv + (vec2<f32>(f32(x), f32(y)) - 1.5) * g.shadow.z, p.z - g.shadow.w);
        }
    }
    // The map ends somewhere: the shadows fade out before it does.
    return mix(1.0, lit / 16.0, 1.0 - smoothstep(0.8, 1.0, edge));
}
