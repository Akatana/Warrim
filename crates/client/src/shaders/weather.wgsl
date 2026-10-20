// The weather: the original's vertex programs rain.bls, snowpoint.bls, sand.bls and patter.bls, one particle (an instance) at a time. A particle
// has a place `pos` at its birth `t0`, a velocity `vel` and a death `t1` (see weather.rs); where it is now is worked out here.

struct Weather {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // x = the weather's clock in seconds, y = the window's height over 768, z = its width times 0.0025
    clock: vec4<f32>,
    fog_color: vec4<f32>,
    // x = start, y = end
    fog: vec4<f32>,
    // The colour of sand grains and mist.
    tint: vec4<f32>,
    // The window's width and height in pixels.
    view: vec4<f32>,
    // The camera's right and up.
    right: vec4<f32>,
    up: vec4<f32>,
};

@group(0) @binding(0) var<uniform> w: Weather;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Instance {
    @location(0) pos: vec3<f32>,
    @location(1) t0: f32,
    @location(2) vel: vec3<f32>,
    @location(3) t1: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) dist: f32,
};

fn project(p: vec3<f32>) -> vec4<f32> {
    return w.view_proj * vec4<f32>(p - w.eye.xyz, 1.0);
}

// The corners of a square drawn as two triangles.
fn corner(i: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0));
    return c[i];
}

// A square of `px` pixels across, centred on the projection of `p` (what a point sprite is).
fn sprite(p: vec3<f32>, px: f32, i: u32) -> vec4<f32> {
    var clip = project(p);
    let c = corner(i);
    clip.x += c.x * px / w.view.x * clip.w;
    clip.y += c.y * px / w.view.y * clip.w;
    return clip;
}

// rain.bls: a drop is a thin triangle, two corners at its head and one two yards up the way it came from; once the head has landed the tail
// goes on falling until it reaches it.
@vertex
fn vs_rain(@builtin(vertex_index) i: u32, p: Instance) -> VsOut {
    let now = w.clock.x;
    let tail = i >= 2u;
    let end = p.t1 + 2.0 / -p.vel.y;
    let age = min(now, select(p.t1, end, tail)) - p.t0;
    var pos = p.pos + p.vel * age;
    let to_eye = normalize(w.eye.xyz - pos);
    let along = normalize(-p.vel);
    let side = normalize(cross(to_eye, along));
    var across = array<f32, 3>(-0.05, 0.05, 0.0);
    var reach = array<f32, 3>(0.0, 0.0, 2.0);
    pos += along * reach[i] + side * across[i];
    var uv = array<vec2<f32>, 3>(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.5, 0.0));
    var out: VsOut;
    out.clip = project(pos);
    out.uv = uv[i];
    // Half see-through, like the original's texture factor of 0x80.
    out.color = vec4<f32>(1.0, 1.0, 1.0, select(0.0, 0.5, p.t0 < now && now < end));
    out.dist = distance(pos, w.eye.xyz);
    return out;
}

// snowpoint.bls: a flake is a point of 14 pixels, smaller the further it is, that fades in over a second and out over a quarter of one.
@vertex
fn vs_snow(@builtin(vertex_index) i: u32, p: Instance) -> VsOut {
    let now = w.clock.x;
    let age = min(now, p.t1) - p.t0;
    let pos = p.pos + p.vel * age;
    let dist = distance(pos, w.eye.xyz);
    let size = max(1.0, clamp(dist * -0.02 + 1.0, 0.0, 1.0) * 14.0 * w.clock.y);
    let fade_out = clamp(1.0 - 4.0 * max(0.0, now - p.t1), 0.0, 1.0);
    var out: VsOut;
    out.clip = sprite(pos, size, i);
    out.uv = corner(i) * 0.5 + 0.5;
    out.color = vec4<f32>(1.0, 1.0, 1.0, clamp(select(age, fade_out, fade_out < 1.0), 0.0, 1.0));
    out.dist = dist;
    return out;
}

// sand.bls: a grain is a small square, coloured by the light, fading in and out over a fifth of a second each.
@vertex
fn vs_sand(@builtin(vertex_index) i: u32, p: Instance) -> VsOut {
    let now = w.clock.x;
    let life = p.t1 - p.t0;
    let age = max(0.0, min(life, now - p.t0));
    let pos = p.pos + p.vel * age;
    let dist = distance(pos, w.eye.xyz);
    let size = max(1.0, clamp(dist * -0.05 + 1.5, 0.0, 1.0) * w.clock.z);
    var out: VsOut;
    out.clip = sprite(pos, size, i);
    out.uv = vec2<f32>(0.5, 0.5);
    out.color = vec4<f32>(w.tint.rgb, clamp(5.0 * age, 0.0, 1.0) * clamp(5.0 * (life - age), 0.0, 1.0));
    out.dist = dist;
    return out;
}

// patter.bls: a splash of rain on the ground, a quarter of a second long, four pictures of it across the atlas, and the row of pictures
// by how steeply the camera looks down on it (row 0 from straight above, row 3 from the side). It is a triangle turned to the camera, its base
// 0.17 yards wide and its tip 0.17 up, over the triangle of the picture's cell given by patter's constants; looking down it is moved a
// little down the camera's up and up the world's.
@vertex
fn vs_splash(@builtin(vertex_index) i: u32, p: Instance) -> VsOut {
    let now = w.clock.x;
    var offset = array<vec3<f32>, 3>(-w.right.xyz * 0.0833, w.up.xyz * 0.1667, w.right.xyz * 0.0833);
    var corner_uv = array<vec2<f32>, 3>(vec2<f32>(0.0, 0.25), vec2<f32>(0.125, 0.04296875), vec2<f32>(0.25, 0.25));
    var rel = p.pos + offset[i] - w.eye.xyz;
    let dist = length(rel);
    let down = max(-rel.y / dist, 0.0);
    rel = rel - w.up.xyz * 0.1667 * (down * 0.5);
    rel.y += down * 0.0833;
    let frame = floor(max(now - p.t0, 0.0) * 4.0 * 3.99);
    let row = floor((1.0 - down) * 3.99);
    var out: VsOut;
    out.clip = w.view_proj * vec4<f32>(rel, 1.0);
    out.uv = vec2<f32>(frame, row) * 0.25 + corner_uv[i];
    out.color = vec4<f32>(1.0, 1.0, 1.0, select(0.0, 0.5, p.t0 < now && now < p.t1));
    out.dist = dist;
    return out;
}

struct Mist {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

// Mist: pictures of fog drifting by, built on the CPU.
@vertex
fn vs_mist(m: Mist) -> VsOut {
    var out: VsOut;
    out.clip = project(m.pos);
    out.uv = m.uv;
    out.color = m.color;
    out.dist = distance(m.pos, w.eye.xyz);
    return out;
}

// Linear fog, as the world's.
fn fogged(rgb: vec3<f32>, dist: f32) -> vec3<f32> {
    let f = clamp((dist - w.fog.x) / max(w.fog.y - w.fog.x, 1.0), 0.0, 1.0);
    return mix(rgb, w.fog_color.rgb, f);
}

@fragment
fn fs_tex(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv) * in.color;
    return vec4<f32>(fogged(t.rgb, in.dist), t.a);
}

@fragment
fn fs_flat(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(fogged(in.color.rgb, in.dist), in.color.a);
}
