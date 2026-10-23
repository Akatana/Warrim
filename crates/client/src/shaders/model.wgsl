// Static (bind-pose) M2 models, instanced per tile.

struct TileUniform {
    offset: vec4<f32>,
};

struct Params {
    // x = alpha-test threshold (0 = off), y = 1 for unlit batches, z = 1 for particles (the vertex colour tints and fades them)
    v: vec4<f32>,
    // The colour (rgb) and opacity of the batch at this moment, and two rows of the matrix that moves its texture.
    tint: vec4<f32>,
    uv0: vec4<f32>,
    uv1: vec4<f32>,
    // The colour a building's batch glows with at night (rgb).
    glow: vec4<f32>,
};

@group(1) @binding(0) var<uniform> tile: TileUniform;
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;
@group(2) @binding(2) var<uniform> params: Params;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(7) color: vec4<f32>,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    // The light of the room the model stands in: ambient (w = 1 inside a room) and the light from the sun's direction.
    @location(8) room_ambient: vec4<f32>,
    @location(9) room_sun: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) room_ambient: vec4<f32>,
    @location(5) room_sun: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let model = mat4x4<f32>(in.m0, in.m1, in.m2, in.m3);
    var out: VsOut;
    let rel = (model * vec4<f32>(in.pos, 1.0)).xyz + tile.offset.xyz;
    out.clip = g.view_proj * vec4<f32>(rel, 1.0);
    out.rel = rel;
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    out.color = in.color;
    out.room_ambient = in.room_ambient;
    out.room_sun = in.room_sun;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(dot(params.uv0.xyz, vec3<f32>(in.uv, 1.0)), dot(params.uv1.xyz, vec3<f32>(in.uv, 1.0)));
    let c = textureSample(tex, samp, uv);
    // A batch whose transparency track says 0 is not there (the spell-focus models, forge and anvil enablers, are such planes); the opaque
    // and alpha-key passes do not blend, so the opacity has to be honoured here.
    if (c.a < params.v.x || params.tint.a <= 0.0) {
        discard;
    }

    // How a batch is lit (params.uv0.w), as in WoW.exe for the batches of a building group (the vertex colour is the baked colour, after its
    // fade at the doors):
    // 0 a model of the world, by the sun, the side facing the camera so that thin double-sided geometry (leaves) is lit both ways;
    // 1 by the baked colour alone, brighter by its alpha (the interior batches, MapObjOverbright.bls);
    // 2 by both, the sun by the alpha and the baked colour by what is left of it (the batches at a door, MapObjTransDiffuse.bls drawn twice);
    // 3 by the sun as a fixed-function light does it, the surface as the file has it, with the baked colour as the colour of the surface.
    let baked = params.uv0.w;
    let building = baked > 1.5;
    var n = normalize(in.normal);
    if (!building && dot(n, in.rel) > 0.0) {
        n = -n;
    }
    let ndl = max(dot(n, -g.sun_dir.xyz), 0.0);
    // Only what the sun lights outdoors takes shadows (rooms and unlit batches are lit by other means).
    var shade = 1.0;
    if (in.room_ambient.a < 0.5 && (baked < 0.5 || building) && params.v.y < 0.5) {
        // The normal that faces the sun, not the camera: the lookup must not change with the view.
        let ns = normalize(in.normal);
        shade = shadow_at(in.rel, select(-ns, ns, dot(ns, -g.sun_dir.xyz) > 0.0));
    }
    // The windows of a building (params.uv1.w) get a little less of the sun's light, 16 of 255. What glows at night (params.glow) is added
    // to the light, and the light of a building stops at 1 as a vertex colour does.
    let sun_color = max(g.sun_color.rgb - params.uv1.w * (16.0 / 255.0), vec3<f32>(0.0));
    let sun = select(
        min(g.ambient.rgb + sun_color * ndl * shade * in.color.rgb, vec3<f32>(1.3)),
        min(g.ambient.rgb + sun_color * ndl * shade * in.color.rgb + params.glow.rgb * g.sun_color.w, vec3<f32>(1.0)),
        building,
    );
    // A model in a room of a building is lit by the colour of its place instead of the sun's: ambient, and the light from the sun's direction.
    // The vertex colours of a model near the lamps of the building carry their light (room_sun.a = 1).
    let room = min(in.room_ambient.rgb + in.room_sun.rgb * ndl + select(vec3<f32>(0.0), in.color.rgb, in.room_sun.a > 0.5), vec3<f32>(1.3));
    // The pixel shader's steps, each clamped to 0..1 as ps_1_x does: v * a, then 4 * t * that, then t * v + that.
    let overbright = clamp(c.rgb * in.color.rgb + clamp(4.0 * c.rgb * (in.color.rgb * in.color.a), vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.0), vec3<f32>(1.0));
    var surface = c.rgb * sun;
    if (in.room_ambient.a > 0.5) {
        surface = c.rgb * room;
    } else if (baked > 1.5 && baked < 2.5) {
        surface = c.rgb * mix(in.color.rgb, sun, in.color.a);
    } else if (baked > 0.5 && baked < 1.5) {
        surface = overbright;
    }
    let lit = select(surface, c.rgb, params.v.y > 0.5) * params.tint.rgb;

    let dist = length(in.rel);
    let f = clamp((dist - g.fog_params.x) / (g.fog_params.y - g.fog_params.x), 0.0, 1.0);
    // A picture that multiplies what is behind it fades into the fog to what changes nothing (1, or one half when it is taken twice).
    let neutral = select(vec3<f32>(1.0), vec3<f32>(0.5), params.v.w > 1.5);
    if (params.v.z > 0.5) {
        // A particle: its colour and its fade, thinner in the fog.
        if (params.v.w > 0.5) {
            return vec4<f32>(mix(c.rgb * in.color.rgb, neutral, f), 1.0);
        }
        return vec4<f32>(c.rgb * in.color.rgb, c.a * in.color.a * (1.0 - f));
    }
    if (params.v.w > 0.5) {
        return vec4<f32>(mix(lit, neutral, f), 1.0);
    }
    return vec4<f32>(mix(lit, g.fog_color.rgb, f), select(c.a, 1.0, params.v.x < 0.0) * params.tint.a);
}
