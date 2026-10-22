//! M2 models, version 256 (1.12): vertices, the first LOD view with its submeshes and texture
//! batches, texture names and render flags, plus the skeleton (bones with keyframed tracks),
//! animation sequences and attachment points for animated models.
//!
//! Model space: x east, y north, z up.

use crate::reader::Reader;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub struct M2Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Bone indices and their weights (out of 255).
    pub bones: [u8; 4],
    pub weights: [u8; 4],
}

/// A keyframed value over time (`AnimationBlock`, 28 bytes in v256).
#[derive(Clone, Debug)]
pub struct M2Track<T> {
    /// 0 none, 1 linear, 2 hermite, 3 bezier (the in/out tangents of 2 and 3 are not used).
    pub interpolation: u16,
    /// Index into `M2::global_sequences`, or -1 when the track follows the animation sequences.
    pub global_sequence: i16,
    /// Per sequence: (first key, last key) into `times` / `keys`.
    pub ranges: Vec<(u32, u32)>,
    /// Milliseconds on the model's timeline: each sequence's keys lie between its start and end.
    pub times: Vec<u32>,
    pub keys: Vec<T>,
}

impl<T> Default for M2Track<T> {
    fn default() -> Self {
        Self { interpolation: 0, global_sequence: -1, ranges: Vec::new(), times: Vec::new(), keys: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct M2Bone {
    pub parent: i16,
    pub flags: u32,
    pub pivot: [f32; 3],
    pub translation: M2Track<[f32; 3]>,
    /// Unit quaternion (x, y, z, w).
    pub rotation: M2Track<[f32; 4]>,
    pub scale: M2Track<[f32; 3]>,
}

#[derive(Clone, Copy, Debug)]
pub struct M2Sequence {
    /// Animation id (0 stand, 4 walk, 5 run, ...); `variation` tells apart several of the same id.
    pub id: u16,
    pub variation: u16,
    /// Start and end on the model's timeline, in milliseconds.
    pub start: u32,
    pub end: u32,
    pub move_speed: f32,
    pub flags: u32,
    /// Weight (out of 32767) of this variation among the sequences chained from the same animation.
    pub probability: u16,
    /// How many times a played sequence repeats: a random count in this range (0 counts as 1).
    pub replay_min: u32,
    pub replay_max: u32,
    /// Milliseconds the client takes to fade from the previous animation into this one.
    pub blend_ms: u32,
    /// Index of the next variation of the same animation, or `None` at the end of the chain.
    pub next_variation: Option<u16>,
}

/// A model's event: `$DSL` is a sound the model plays all the time (a waterfall, a fountain, a mill), `data` the number of its entry in
/// `SoundEntries.dbc`, at `position` on `bone` (the position is added to the bone's pivot).
#[derive(Clone, Copy, Debug)]
pub struct M2Event {
    pub id: [u8; 4],
    pub data: u32,
    pub bone: u32,
    pub position: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct M2Attachment {
    /// 0 shield, 1 right hand, 2 left hand, 11 head, ... (see `M2::attachment_lookup`).
    pub id: u32,
    pub bone: u16,
    pub position: [f32; 3],
}

/// A particle emitter (the vanilla layout, 504 bytes): where it sits, how it throws particles and how they look over their life.
/// The tracks are keyed by the animation sequence like the bones'.
#[derive(Clone, Debug)]
pub struct M2Particle {
    pub flags: u32,
    /// Where it is on its bone, model space.
    pub position: [f32; 3],
    pub bone: u16,
    pub texture: u16,
    /// 0 opaque, 1 alpha key, 2 alpha, 3 and 4 additive.
    pub blend: u8,
    /// 1 a plane, 2 a sphere, 3 a spline.
    pub emitter_type: u8,
    pub particle_type: u8,
    pub head_or_tail: u8,
    /// The texture is cut into this many rows and columns of cells.
    pub rows: u16,
    pub cols: u16,
    pub speed: M2Track<f32>,
    pub variation: M2Track<f32>,
    /// How far from straight up a particle may be thrown, and how far round: radians.
    pub vertical_range: M2Track<f32>,
    pub horizontal_range: M2Track<f32>,
    pub gravity: M2Track<f32>,
    pub lifespan: M2Track<f32>,
    /// Particles a second.
    pub rate: M2Track<f32>,
    pub area_length: M2Track<f32>,
    pub area_width: M2Track<f32>,
    /// The share of the life at which the middle colour and size are reached.
    pub mid: f32,
    /// Colour at the start, the middle and the end of the life, as 0xAARRGGBB.
    pub colors: [u32; 3],
    pub sizes: [f32; 3],
    pub scales: [f32; 3],
    pub slowdown: f32,
    /// How fast the quad of a particle turns about the view direction, radians a second (FUN_007b2a50: the angle is this times its age).
    pub spin: f32,
    /// The cells of the texture a particle shows over its life, in the two halves of it (before and after `mid`): the first and the last cell, and
    /// the power of the share of the life by which it steps from one to the other (0 for the last cell throughout).
    pub head_cells: [(u16, u16, u16); 2],
    /// The same for the tail quad of a particle that has one (types 1 and 2), and how long the tail is (in the speed of the particle times this).
    pub tail_cells: [(u16, u16); 2],
    pub tail_length: f32,
    /// The point below the emitter that the particles of a plane fly away from (0: they fly along the angles).
    pub z_source: M2Track<f32>,
    /// Emitting is on while this track says so (1 on).
    pub enabled: M2Track<u8>,
}

/// What moves the picture on a batch: a colour and its alpha (`M2Color`), a transparency, and the movement of the texture (`M2UvAnim`), each a track.
#[derive(Clone, Debug, Default)]
pub struct M2Color {
    pub color: M2Track<[f32; 3]>,
    pub alpha: M2Track<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct M2UvAnim {
    pub translation: M2Track<[f32; 3]>,
    /// A unit quaternion (x, y, z, w): the texture turns about its middle by the angle about z.
    pub rotation: M2Track<[f32; 4]>,
    pub scale: M2Track<[f32; 3]>,
}

#[derive(Clone, Debug)]
pub struct M2Texture {
    /// 0 = fixed file name; others are replaceable (skins, hair, ...).
    pub kind: u32,
    pub flags: u32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Opaque,
    AlphaKey,
    Alpha,
    Additive,
    /// The picture multiplies what is behind it (blend 5), and twice over (blend 6).
    Modulate,
    Modulate2x,
}

#[derive(Clone, Debug)]
pub struct M2Batch {
    /// Range into `M2::indices`.
    pub index_start: usize,
    pub index_count: usize,
    /// Index into `M2::textures`, if the lookup resolved.
    pub texture: Option<usize>,
    pub blend: Blend,
    pub two_sided: bool,
    pub unlit: bool,
    /// The material's flags as stored (1 unlit, 2 unfogged, 4 two sided, 8 unculled?, 0x10 no depth write) and blend mode.
    pub render_flags: u16,
    pub blend_mode: u16,
    /// What animates it: an entry of `M2::colors`, of `M2::transparency` (through the model's lookup) and of `M2::uv_anims`.
    pub color: Option<usize>,
    pub transparency: Option<usize>,
    pub uv_anim: Option<usize>,
    /// Priority plane; lower draws first among translucent batches.
    pub priority: i8,
    /// Mesh part id of the submesh (character geosets: group * 100 + variant).
    pub submesh_id: u16,
}

pub struct M2 {
    pub name: String,
    pub vertices: Vec<M2Vertex>,
    /// Triangle list indexing `vertices`.
    pub indices: Vec<u16>,
    pub textures: Vec<M2Texture>,
    pub batches: Vec<M2Batch>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Simplified collision mesh (model space): triangle list indexing `collision_vertices`.
    pub collision_vertices: Vec<[f32; 3]>,
    pub collision_indices: Vec<u16>,
    pub bones: Vec<M2Bone>,
    pub sequences: Vec<M2Sequence>,
    pub global_sequences: Vec<u32>,
    pub attachments: Vec<M2Attachment>,
    pub events: Vec<M2Event>,
    /// Attachment id -> index into `attachments` (0xFFFF = none).
    pub attachment_lookup: Vec<u16>,
    pub particles: Vec<M2Particle>,
    pub colors: Vec<M2Color>,
    /// Opacity tracks, 0 to 1.
    pub transparency: Vec<M2Track<f32>>,
    pub uv_anims: Vec<M2UvAnim>,
    pub cameras: Vec<M2Camera>,
}

/// A camera of a model (the scenes behind the character screens and the fly-bys of the race introductions have them): its field of view
/// (radians) and clip distances, and where it stands and what it looks at in model space, still (the base plus the first key of its track)
/// and along the path it flies (base plus the keys of the track: each is the value, then the tangent in and the tangent out).
#[derive(Clone, Debug)]
pub struct M2Camera {
    pub fov: f32,
    pub far: f32,
    pub near: f32,
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub position_base: [f32; 3],
    pub target_base: [f32; 3],
    pub position_path: M2Track<[f32; 9]>,
    pub target_path: M2Track<[f32; 9]>,
}

/// `(count, offset)` pair used throughout the header.
fn array(r: &mut Reader) -> Result<(usize, usize)> {
    Ok((r.u32()? as usize, r.u32()? as usize))
}

fn slice<'a>(data: &'a [u8], (n, ofs): (usize, usize), stride: usize, what: &str) -> Result<&'a [u8]> {
    let len = n.checked_mul(stride).ok_or_else(|| Error::Invalid(format!("{what}: size overflow")))?;
    data.get(ofs..ofs.checked_add(len).unwrap_or(usize::MAX))
        .ok_or_else(|| Error::Invalid(format!("{what}: {n} x {stride} bytes at {ofs} is outside the file")))
}

fn u16s(b: &[u8]) -> Vec<u16> {
    b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn f32s<const N: usize>(b: &[u8]) -> [f32; N] {
    std::array::from_fn(|i| f32::from_bits(le32(&b[i * 4..])))
}

/// Decode one `AnimationBlock` at `b`. A track that does not hold together comes back empty
/// (the bone then keeps its bind pose) instead of failing the whole model.
fn track<T>(data: &[u8], b: &[u8], key_size: usize, key: impl Fn(&[u8]) -> T) -> M2Track<T> {
    let (n_ranges, ofs_ranges) = (le32(&b[4..]) as usize, le32(&b[8..]) as usize);
    let (n_times, ofs_times) = (le32(&b[12..]) as usize, le32(&b[16..]) as usize);
    let (n_keys, ofs_keys) = (le32(&b[20..]) as usize, le32(&b[24..]) as usize);
    let parts = (
        slice(data, (n_ranges * 2, ofs_ranges), 4, "track ranges"),
        slice(data, (n_times, ofs_times), 4, "track times"),
        slice(data, (n_keys, ofs_keys), key_size, "track keys"),
    );
    let (Ok(ranges), Ok(times), Ok(keys)) = parts else { return M2Track::default() };
    M2Track {
        interpolation: u16::from_le_bytes([b[0], b[1]]),
        global_sequence: i16::from_le_bytes([b[2], b[3]]),
        ranges: ranges.chunks_exact(8).map(|r| (le32(r), le32(&r[4..]))).collect(),
        times: times.chunks_exact(4).map(le32).collect(),
        keys: keys.chunks_exact(key_size).map(key).collect(),
    }
}

fn cstr(data: &[u8], ofs: usize, len: usize) -> String {
    let s = data.get(ofs..ofs.saturating_add(len)).unwrap_or(&[]);
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).into_owned()
}

impl M2 {
    pub fn parse(data: &[u8]) -> Result<M2> {
        let mut r = Reader::new(data);
        let magic = r.bytes(4)?;
        if magic != b"MD20" {
            return Err(Error::Magic { expected: "MD20", found: String::from_utf8_lossy(magic).into() });
        }
        let version = r.u32()?;
        if version != 256 {
            return Err(Error::Unsupported(format!("M2 version {version}, expected 256")));
        }
        let (name_len, name_ofs) = array(&mut r)?;
        let name = cstr(data, name_ofs, name_len);
        r.skip(4)?; // global flags
        let global_seq = array(&mut r)?;
        let sequences = array(&mut r)?;
        let _seq_lookup = array(&mut r)?;
        let _playable_anims = array(&mut r)?; // 4-byte entries; present in 1.x headers
        let bones = array(&mut r)?;
        let _key_bones = array(&mut r)?;
        let verts = array(&mut r)?;
        let views = array(&mut r)?;
        let colors = array(&mut r)?;
        let textures = array(&mut r)?;
        let transparency = array(&mut r)?;
        let _unknown = array(&mut r)?; // an extra array of the 1.12 header, empty in the models looked at
        let uv_anim = array(&mut r)?;
        let _tex_replace = array(&mut r)?; // small u32 array, seen in every 1.12 model
        let materials = array(&mut r)?;
        let _bone_lookup = array(&mut r)?;
        let tex_lookup = array(&mut r)?;
        let _tex_units = array(&mut r)?;
        let trans_lookup = array(&mut r)?;
        let uv_lookup = array(&mut r)?;
        // bounding box (24) + radius (4), collision box (24) + radius (4), then the collision mesh.
        r.skip(56)?;
        let coll_indices = array(&mut r)?;
        let coll_verts = array(&mut r)?;
        let _coll_normals = array(&mut r)?;
        let attachments = array(&mut r)?;
        let attach_lookup = array(&mut r)?;
        let events = array(&mut r)?;
        let _lights = array(&mut r)?;
        let cameras = array(&mut r)?;
        let _camera_lookup = array(&mut r)?;
        let _ribbons = array(&mut r)?;
        let particles = array(&mut r)?;

        // Vertices: pos(12) bone weights(4) bone indices(4) normal(12) uv0(8) uv1(8).
        let vbytes = slice(data, verts, 48, "vertices")?;
        let mut vertices = Vec::with_capacity(verts.0);
        for v in vbytes.chunks_exact(48) {
            let mut vr = Reader::new(v);
            let pos = vr.vec3()?;
            let weights = [v[12], v[13], v[14], v[15]];
            let bones = [v[16], v[17], v[18], v[19]];
            vr.skip(8)?;
            let normal = vr.vec3()?;
            vertices.push(M2Vertex { pos, normal, uv: [vr.f32()?, vr.f32()?], bones, weights });
        }

        let textures_out = {
            let tbytes = slice(data, textures, 16, "textures")?;
            tbytes
                .chunks_exact(16)
                .map(|t| {
                    let mut tr = Reader::new(t);
                    let (kind, flags, len, ofs) = (tr.u32()?, tr.u32()?, tr.u32()? as usize, tr.u32()? as usize);
                    Ok(M2Texture { kind, flags, name: cstr(data, ofs, len) })
                })
                .collect::<Result<Vec<_>>>()?
        };
        let tex_lookup = u16s(slice(data, tex_lookup, 2, "texture lookup")?);
        let trans_lookup = slice(data, trans_lookup, 2, "transparency lookup").map(u16s).unwrap_or_default();
        let uv_lookup = slice(data, uv_lookup, 2, "texture animation lookup").map(u16s).unwrap_or_default();
        let fixed = |k: &[u8]| i16::from_le_bytes([k[0], k[1]]) as f32 / 32767.0;
        let colors_out: Vec<M2Color> = slice(data, colors, 56, "colours")
            .map(|b| b.chunks_exact(56).map(|c| M2Color { color: track(data, &c[0..28], 12, f32s::<3>), alpha: track(data, &c[28..56], 2, fixed) }).collect())
            .unwrap_or_default();
        let transparency_out: Vec<M2Track<f32>> =
            slice(data, transparency, 28, "transparency").map(|b| b.chunks_exact(28).map(|t| track(data, t, 2, fixed)).collect()).unwrap_or_default();
        let uv_anims: Vec<M2UvAnim> = slice(data, uv_anim, 84, "texture animations")
            .map(|b| {
                b.chunks_exact(84)
                    .map(|u| M2UvAnim { translation: track(data, &u[0..28], 12, f32s::<3>), rotation: track(data, &u[28..56], 16, f32s::<4>), scale: track(data, &u[56..84], 12, f32s::<3>) })
                    .collect()
            })
            .unwrap_or_default();
        let materials: Vec<(u16, u16)> = slice(data, materials, 4, "render flags")?
            .chunks_exact(4)
            .map(|c| (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]])))
            .collect();

        if views.0 == 0 {
            return Err(Error::Invalid("M2 has no views".into()));
        }
        // View 0 (highest detail): vertex lookup, triangles, properties, submeshes, texture units.
        let vb = slice(data, views, 44, "views")?;
        let mut vr = Reader::new(vb);
        let lookup = u16s(slice(data, array(&mut vr)?, 2, "view index lookup")?);
        let tris = u16s(slice(data, array(&mut vr)?, 2, "view triangles")?);
        let _props = array(&mut vr)?;
        let (n_sub, ofs_sub) = array(&mut vr)?;
        let (n_batch, ofs_batch) = array(&mut vr)?;

        let mut indices = Vec::with_capacity(tris.len());
        for &t in &tris {
            let v = *lookup.get(t as usize).ok_or_else(|| Error::Invalid("triangle index outside lookup".into()))?;
            if v as usize >= vertices.len() {
                return Err(Error::Invalid("lookup index outside vertices".into()));
            }
            indices.push(v);
        }

        // Submeshes (32 bytes): id, level, vstart, vcount, istart, icount, ... centre.
        let subs: Vec<(usize, usize, u16)> = slice(data, (n_sub, ofs_sub), 32, "submeshes")?
            .chunks_exact(32)
            .map(|s| {
                let g = |o: usize| u16::from_le_bytes([s[o], s[o + 1]]) as usize;
                (g(8), g(10), g(0) as u16)
            })
            .collect();

        let mut batches = Vec::new();
        for b in slice(data, (n_batch, ofs_batch), 24, "texture units")?.chunks_exact(24) {
            let g = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
            let (flags, priority) = (b[0], b[1] as i8);
            // M2Batch (24 bytes): flags, priority, shader, submesh@4, geoset, colour, material@10, layer, count, texture combo@16, ...
            let (sub, material, tex_combo) = (g(4), g(10), g(16));
            let (color, transparency, uv_anim) = (
                Some(g(8)).filter(|&c| c < colors_out.len()),
                trans_lookup.get(g(20)).map(|&t| t as usize).filter(|&t| t < transparency_out.len()),
                uv_lookup.get(g(22)).map(|&t| t as usize).filter(|&t| t < uv_anims.len()),
            );
            let Some(&(index_start, index_count, submesh_id)) = subs.get(sub) else { continue };
            if index_start + index_count > indices.len() {
                continue;
            }
            let (mflags, blend) = materials.get(material).copied().unwrap_or((0, 0));
            let texture = tex_lookup.get(tex_combo).map(|&t| t as usize).filter(|&t| t < textures_out.len());
            batches.push(M2Batch {
                index_start,
                index_count,
                texture,
                blend: match blend {
                    0 => Blend::Opaque,
                    1 => Blend::AlphaKey,
                    3 | 4 => Blend::Additive,
                    5 => Blend::Modulate,
                    6 => Blend::Modulate2x,
                    _ => Blend::Alpha,
                },
                two_sided: mflags & 0x04 != 0,
                unlit: mflags & 0x01 != 0,
                render_flags: mflags,
                blend_mode: blend,
                color,
                transparency,
                uv_anim,
                priority,
                submesh_id,
            });
            let _ = flags;
        }

        let mut bounds_min = [f32::MAX; 3];
        let mut bounds_max = [f32::MIN; 3];
        for v in &vertices {
            for i in 0..3 {
                bounds_min[i] = bounds_min[i].min(v.pos[i]);
                bounds_max[i] = bounds_max[i].max(v.pos[i]);
            }
        }
        // The collision mesh is optional: ignore it if it does not hold together.
        let collision_vertices: Vec<[f32; 3]> = slice(data, coll_verts, 12, "collision vertices")
            .map(|b| {
                b.chunks_exact(12)
                    .map(|v| [f32::from_bits(u32::from_le_bytes([v[0], v[1], v[2], v[3]])), f32::from_bits(u32::from_le_bytes([v[4], v[5], v[6], v[7]])), f32::from_bits(u32::from_le_bytes([v[8], v[9], v[10], v[11]]))])
                    .collect()
            })
            .unwrap_or_default();
        let collision_indices: Vec<u16> = slice(data, coll_indices, 2, "collision indices").map(u16s).unwrap_or_default();
        let (collision_vertices, collision_indices) = if collision_indices.len() % 3 == 0
            && collision_indices.iter().all(|&i| (i as usize) < collision_vertices.len())
        {
            (collision_vertices, collision_indices)
        } else {
            (Vec::new(), Vec::new())
        };

        let global_sequences = slice(data, global_seq, 4, "global sequences").map(|b| b.chunks_exact(4).map(le32).collect()).unwrap_or_default();
        let sequences = slice(data, sequences, 68, "sequences")
            .map(|b| {
                b.chunks_exact(68)
                    .map(|s| M2Sequence {
                        id: u16::from_le_bytes([s[0], s[1]]),
                        variation: u16::from_le_bytes([s[2], s[3]]),
                        start: le32(&s[4..]),
                        end: le32(&s[8..]),
                        move_speed: f32::from_bits(le32(&s[12..])),
                        flags: le32(&s[16..]),
                        probability: u16::from_le_bytes([s[20], s[21]]),
                        replay_min: le32(&s[24..]),
                        replay_max: le32(&s[28..]),
                        blend_ms: le32(&s[32..]),
                        next_variation: Some(u16::from_le_bytes([s[64], s[65]])).filter(|&n| n != 0xFFFF),
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Bones (108 bytes): key bone, flags, parent, mesh part, translation / rotation / scale tracks, pivot.
        let bones = slice(data, bones, 108, "bones")
            .map(|b| {
                b.chunks_exact(108)
                    .map(|b| M2Bone {
                        flags: le32(&b[4..]),
                        parent: i16::from_le_bytes([b[8], b[9]]),
                        translation: track(data, &b[12..40], 12, f32s::<3>),
                        rotation: track(data, &b[40..68], 16, f32s::<4>),
                        scale: track(data, &b[68..96], 12, f32s::<3>),
                        pivot: f32s::<3>(&b[96..108]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Attachments (48 bytes): id, bone, unknown, position, visibility track.
        let attachments = slice(data, attachments, 48, "attachments")
            .map(|b| {
                b.chunks_exact(48)
                    .map(|a| M2Attachment { id: le32(a), bone: u16::from_le_bytes([a[4], a[5]]), position: f32s::<3>(&a[8..20]) })
                    .collect()
            })
            .unwrap_or_default();
        let attachment_lookup = slice(data, attach_lookup, 2, "attachment lookup").map(u16s).unwrap_or_default();
        let events: Vec<M2Event> = slice(data, events, 44, "events")
            .map(|b| b.chunks_exact(44).map(|e| M2Event { id: [e[0], e[1], e[2], e[3]], data: le32(&e[4..]), bone: le32(&e[8..]), position: f32s::<3>(&e[12..24]) }).collect())
            .unwrap_or_default();

        // Particle emitters (504 bytes each): id, flags, position, bone, texture, two names, blend and shape bytes, texture cells,
        // ten tracks, then the colours and sizes over the life and a last track.
        let particles = slice(data, particles, 504, "particle emitters")
            .map(|b| {
                b.chunks_exact(504)
                    .map(|e| {
                        let t = |o: usize| track(data, &e[o..o + 28], 4, |k| f32::from_bits(le32(k)));
                        M2Particle {
                            flags: le32(&e[4..]),
                            position: f32s::<3>(&e[8..20]),
                            bone: u16::from_le_bytes([e[20], e[21]]),
                            texture: u16::from_le_bytes([e[22], e[23]]),
                            blend: e[40],
                            emitter_type: e[42],
                            particle_type: e[44],
                            head_or_tail: e[45],
                            rows: u16::from_le_bytes([e[48], e[49]]).max(1),
                            cols: u16::from_le_bytes([e[50], e[51]]).max(1),
                            speed: t(52),
                            variation: t(80),
                            vertical_range: t(108),
                            horizontal_range: t(136),
                            gravity: t(164),
                            lifespan: t(192),
                            rate: t(220),
                            area_length: t(248),
                            area_width: t(276),
                            mid: f32::from_bits(le32(&e[332..])),
                            colors: [le32(&e[336..]), le32(&e[340..]), le32(&e[344..])],
                            sizes: f32s::<3>(&e[348..360]),
                            scales: f32s::<3>(&e[392..404]),
                            slowdown: f32::from_bits(le32(&e[404..])),
                            spin: f32::from_bits(le32(&e[0x198..])),
                            head_cells: {
                                let u = |o: usize| u16::from_le_bytes([e[o], e[o + 1]]);
                                [(u(0x168), u(0x16a), u(0x16c)), (u(0x16e), u(0x170), u(0x172))]
                            },
                            tail_cells: {
                                let u = |o: usize| u16::from_le_bytes([e[o], e[o + 1]]);
                                [(u(0x174), u(0x176)), (u(0x178), u(0x17a))]
                            },
                            tail_length: f32::from_bits(le32(&e[0x17c..])),
                            z_source: t(304),
                            enabled: track(data, &e[476..504], 1, |k| k[0]),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let cameras = slice(data, cameras, 124, "cameras")
            .map(|b| {
                b.chunks_exact(124)
                    .map(|c| {
                        // A path that curves (hermite or bezier) has the tangents in its keys.
                        let path = |b: &[u8]| {
                            let size = if u16::from_le_bytes([b[0], b[1]]) >= 2 { 36 } else { 12 };
                            track(data, b, size, |k| std::array::from_fn(|i| k.get(i * 4..i * 4 + 4).map_or(0.0, |v| f32::from_bits(le32(v)))))
                        };
                        let (pos, target) = (path(&c[16..44]), path(&c[56..84]));
                        let (pos_base, target_base) = (f32s::<3>(&c[44..]), f32s::<3>(&c[84..]));
                        let moved = |base: [f32; 3], t: &M2Track<[f32; 9]>| t.keys.first().map_or(base, |k| [base[0] + k[0], base[1] + k[1], base[2] + k[2]]);
                        M2Camera {
                            fov: f32s::<1>(&c[4..])[0],
                            far: f32s::<1>(&c[8..])[0],
                            near: f32s::<1>(&c[12..])[0],
                            position: moved(pos_base, &pos),
                            target: moved(target_base, &target),
                            position_base: pos_base,
                            target_base,
                            position_path: pos,
                            target_path: target,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(M2 {
            name,
            cameras,
            vertices,
            indices,
            textures: textures_out,
            batches,
            bounds_min,
            bounds_max,
            collision_vertices,
            collision_indices,
            bones,
            sequences,
            global_sequences,
            attachments,
            events,
            attachment_lookup,
            particles,
            colors: colors_out,
            transparency: transparency_out,
            uv_anims,
        })
    }
}
