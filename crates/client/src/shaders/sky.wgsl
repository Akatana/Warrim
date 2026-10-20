// Sky gradient: fog colour at the horizon, up through the middle band to the top colour.

struct Globals {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    sky_top: vec4<f32>,
    sky_mid: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One oversized triangle covering the screen.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: VsOut;
    out.clip = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let w = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let e = clamp(normalize(w.xyz / w.w).y, 0.0, 1.0);
    let low = mix(g.fog_color.rgb, g.sky_mid.rgb, smoothstep(0.0, 0.25, e));
    return vec4<f32>(mix(low, g.sky_top.rgb, smoothstep(0.2, 0.9, e)), 1.0);
}
