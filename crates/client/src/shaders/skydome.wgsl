// The sky's dome, stars, sun and moon, and clouds. Every vertex is a direction from the camera, drawn at the far depth.

struct Sky {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> sky: Sky;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) dir: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VsOut {
    var out: VsOut;
    var clip = sky.view_proj * vec4<f32>(pos, 0.0);
    clip.z = clip.w;
    out.clip = clip;
    out.uv = uv;
    out.color = color;
    out.dir = pos;
    return out;
}

@fragment
fn fs_flat(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}

@fragment
fn fs_tex(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv) * in.color;
}

// Where on the cloud picture a direction is: the picture lies flat over the cap of the cloud dome, which is centred below the camera so that
// its rim is the horizon. A direction at elevation e meets the cap at angle acos(cos(45 degrees) * cos e) - e from its apex, and the picture
// is laid out by the dome's rings (as many texels from the middle as rings from the apex).
fn cloud_uv(d: vec3<f32>) -> vec2<f32> {
    // Render space (x east, y up, z south) to the original's (x north, y west, z up).
    let n = vec3<f32>(-d.z, -d.x, d.y);
    let rho = length(n.xy);
    let e = atan2(n.z, rho);
    let lat = (acos(0.70710678 * cos(e)) - e) / 3.14159265;
    var rings = array<f32, 12>(0.0, 0.025, 0.05, 0.075, 0.1, 0.125, 0.15, 0.175, 0.205, 0.23, 0.245, 0.25);
    var ring = 11.0;
    for (var i = 1; i < 12; i++) {
        if (lat <= rings[i]) {
            ring = f32(i - 1) + (lat - rings[i - 1]) / (rings[i] - rings[i - 1]);
            break;
        }
    }
    let bearing = vec2<f32>(n.x, n.y) / max(rho, 1e-5);
    return 0.5 + bearing * (ring / 11.0 * 0.5);
}

@fragment
fn fs_cloud(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, cloud_uv(in.dir));
    return vec4<f32>(c.rgb, c.a * in.color.a);
}
