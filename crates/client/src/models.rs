//! CPU-side M2 models: parsed once, shared between tiles and worker threads.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wowfmt::blp::Blp;
use wowfmt::m2::{Blend, M2};
use wowfmt::wmo::{group_path, BatchKind, PortalRef, WmoDoodad, WmoGroup, WmoLiquid, WmoPortal, WmoRoot, LIQUID_CELL};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ModelVertex {
    /// Render space (x east, y up, z south), model-local.
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Baked lighting of a building group (`MOCV`, after the fade at the doors): rgb is the light, and a what the batch's `Baked` does with
    /// it. White with a = 0 for everything without baked lighting.
    pub color: [u8; 4],
}

/// How a batch of a building group is lit, which is a matter of the batch (see `model.wgsl`).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Baked {
    /// A model of the world: by the sun and the ambient light, the surface turned toward the viewer.
    #[default]
    Sun = 0,
    /// By the baked colours alone, brighter by their alpha.
    Interior = 1,
    /// By the baked colours and the sun, the sun by the alpha.
    Transition = 2,
    /// By the sun and the ambient light, as a fixed-function light does it: with the surface as the file has it, not turned toward the
    /// viewer, so that the inside of a window the sun shines on from outside is dark. The baked colours, if the group has them, are the
    /// colour of the surface.
    Exterior = 3,
}

impl Baked {
    pub fn code(self) -> f32 {
        self as u8 as f32
    }
}

/// WoW.exe's `mapObjOverbright` with pixel shaders on, which they are on current hardware: the interior batches of buildings are lit by
/// `MapObjOverbright.bls` (`model.wgsl`). Off, the original draws them with fixed-function blending, which this does not have a shader for;
/// the colours differ only by `WmoGroup::faded_colors`.
const MAP_OBJECT_OVERBRIGHT: bool = true;

/// Decoded texture of arbitrary size with its stored mip chain (RGBA8).
pub struct RawTexture {
    pub width: u32,
    pub height: u32,
    pub mips: Vec<Vec<u8>>,
}

#[derive(Clone)]
pub struct BatchCpu {
    pub index_start: u32,
    pub index_count: u32,
    /// Cache key (lower-cased path) and pixels; `None` renders flat grey.
    pub texture: Option<(String, Arc<RawTexture>)>,
    pub wrap: bool,
    pub blend: Blend,
    pub unlit: bool,
    pub baked: Baked,
    /// A window of a building (`MOMT` flag 0x20): what the sun lights it with is a little weaker.
    pub window: bool,
    /// The colour it glows with at night (see `sky::window_glow`).
    pub glow: [u8; 3],
    /// Which tracks of the model animate how it looks.
    pub refs: crate::anim::BatchRefs,
    /// Billboards of a particle emitter: the vertex colour tints and fades them (see `model.wgsl`).
    pub particle: bool,
}

pub struct ModelCpu {
    pub key: String,
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u16>,
    pub batches: Vec<BatchCpu>,
    /// Solid triangles for collision (render space, model-local): vertices + triangle list.
    pub col_vertices: Vec<[f32; 3]>,
    pub col_indices: Vec<u32>,
    /// The tracks that animate how its batches look, if any do.
    pub tracks: Option<Arc<crate::anim::LookTracks>>,
}

/// One placed model: cache entry plus a tile-local transform.
pub struct DoodadInstance {
    pub model: Arc<ModelCpu>,
    pub matrix: Mat4,
    /// Drawn with the tile (false for a model that moves: it is shown by `fires.rs` and only its solid parts count here).
    pub drawn: bool,
    /// How it is lit if it stands in a room of a building.
    pub light: Option<InteriorLight>,
}

#[derive(Default)]
pub struct ModelCache {
    models: Mutex<HashMap<String, Option<Arc<ModelCpu>>>>,
    textures: Mutex<HashMap<String, Option<Arc<RawTexture>>>>,
    wmos: Mutex<HashMap<String, Arc<WmoCpu>>>,
    /// What each placed model does besides standing there: its particle emitters (fire, smoke, light shafts) and whether it moves (a flame or a glow
    /// that turns to the viewer, a swinging lamp).
    living: Mutex<HashMap<String, (bool, bool)>>,
    /// The sounds a model plays all the time (`$DSL` events) by model: where they come from (render space, model-local) and their `SoundEntries` number.
    sounds: Mutex<HashMap<String, Vec<(Vec3, u32)>>>,
}

/// How a model in a room of a building is lit: the colour of the ambient light and of the light that comes from the sun's direction, replacing the
/// ones of the open air (red, green and blue, 0 to 1), and whether the model's vertex colours carry the light of the building's lamps near it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteriorLight {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub baked: bool,
}

/// The light a model placed in a room of a building is lit by, from the colour of its place (`MODD`: blue, green, red, alpha as the file has it), as the
/// original works it out: the ambient light is that colour held down to 96 in its brightest channel, the light from the sun's direction is that colour
/// held up to 112.
pub fn interior_light(color: [u8; 4]) -> InteriorLight {
    room_light([color[2], color[1], color[0]], 112.0)
}

/// The light of a place of the colour `rgb` (0 to 255): the ambient light is that colour held down to 96 in its brightest channel, the light from
/// above that colour held up to `floor` (WoW.exe FUN_006a77e0: 112 for what a building lists, 168 for what stands in it on the map).
fn room_light(rgb: [u8; 3], floor: f32) -> InteriorLight {
    let rgb = rgb.map(f32::from);
    let brightest = rgb.iter().cloned().fold(0.0, f32::max);
    if brightest == 0.0 {
        return InteriorLight { ambient: [0.0; 3], diffuse: [floor / 255.0; 3], baked: false };
    }
    InteriorLight { ambient: rgb.map(|c| c * (96.0 / brightest).min(1.0) / 255.0), diffuse: rgb.map(|c| (c * (floor / brightest).max(1.0)).min(255.0) / 255.0), baked: false }
}

/// How a model of the map that stands in a room of a placed building is lit, as WoW.exe does it (FUN_0069e4c0): a line from a yard above it to 12
/// yards below it is cast against the room's triangles, and the baked colour at the nearest hit is the light of the place (`room_light`, up to 168).
/// A triangle with the sun flag (`MOPY` 1) leaves it lit as outdoors (`None`), as does finding no room.
pub fn room_light_at(wmos: &[WmoInstance], at: Vec3) -> Option<InteriorLight> {
    let mut nearest: Option<(f32, Option<[u8; 3]>)> = None;
    for w in wmos {
        let inverse = w.matrix.inverse();
        let (from, to) = (inverse.transform_point3(at + Vec3::Y), inverse.transform_point3(at - Vec3::Y * 12.0));
        let (low, high) = (from.min(to) - 0.5, from.max(to) + 0.5);
        for g in &w.wmo.groups {
            // Rooms only: the groups of the outside (`flags & 0x48`) are lit by the sun.
            let Some(model) = g.model.as_ref().filter(|_| g.flags & 0x48 == 0 && g.flags & 0x2000 != 0) else { continue };
            // The bounds are as the file has them (z up), the model as the renderer has it (y up, z south).
            let (lo, hi) = (Vec3::new(g.bbox.0[0], g.bbox.0[2], -g.bbox.1[1]), Vec3::new(g.bbox.1[0], g.bbox.1[2], -g.bbox.0[1]));
            if hi.cmplt(low).any() || lo.cmpgt(high).any() {
                continue;
            }
            let dir = to - from;
            for (t, tri) in model.indices.chunks_exact(3).enumerate() {
                let v = |i: u16| &model.vertices[i as usize];
                let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
                let (pa, pb, pc) = (Vec3::from(a.pos), Vec3::from(b.pos), Vec3::from(c.pos));
                // Moeller-Trumbore, both sides.
                let (e1, e2) = (pb - pa, pc - pa);
                let h = dir.cross(e2);
                let det = e1.dot(h);
                if det.abs() < 1e-8 {
                    continue;
                }
                let s = from - pa;
                let u = s.dot(h) / det;
                let q = s.cross(e1);
                let vv = dir.dot(q) / det;
                let d = e2.dot(q) / det;
                if u < 0.0 || vv < 0.0 || u + vv > 1.0 || !(0.0..=1.0).contains(&d) || nearest.is_some_and(|(n, _)| n <= d) {
                    continue;
                }
                let colour = (!g.sunlit.get(t).copied().unwrap_or(false)).then(|| {
                    let mix = |k: usize| (a.color[k] as f32 * (1.0 - u - vv) + b.color[k] as f32 * u + c.color[k] as f32 * vv) as u8;
                    [mix(0), mix(1), mix(2)]
                });
                nearest = Some((d, colour));
            }
        }
    }
    nearest.and_then(|(_, colour)| colour).map(|rgb| room_light(rgb, 168.0))
}

/// A model of the world that has particle emitters or moves, and must be shown by `fires.rs` instead of (or besides) the still model of the tile.
pub struct Living {
    /// The model's file.
    pub file: String,
    /// It moves: its mesh is shown by `fires.rs` too (and the tile leaves it out).
    pub mesh: bool,
}

/// One group of a building, indexed like the group files. `model` is `None` for groups without
/// geometry or that failed to load.
#[derive(Default)]
pub struct WmoGroupCpu {
    /// The group's number in `WMOAreaTable`.
    pub id: u32,
    pub flags: u32,
    /// Model-space bounds.
    pub bbox: ([f32; 3], [f32; 3]),
    /// Range of this group's entries in `WmoCpu::portal_refs`.
    pub portals: (u16, u16),
    pub model: Option<Arc<ModelCpu>>,
    /// For each triangle, whether it is lit by the sun (`MOPY` flag 1).
    pub sunlit: Vec<bool>,
}

/// One placed building: cache entry plus a tile-local transform.
pub struct WmoInstance {
    pub wmo: Arc<WmoCpu>,
    pub matrix: Mat4,
}

/// A loaded building: its groups, portals and the furniture lists of its doodad sets.
#[derive(Default)]
pub struct WmoCpu {
    /// The building's number in `WMOAreaTable`.
    pub id: u32,
    pub groups: Vec<WmoGroupCpu>,
    pub portal_verts: Vec<[f32; 3]>,
    pub portals: Vec<WmoPortal>,
    pub portal_refs: Vec<PortalRef>,
    pub doodads: Vec<WmoDoodad>,
    pub doodad_sets: Vec<(u32, u32)>,
    /// The doodads that stand in rooms (indoor groups), which are lit by their own light and not the sun's.
    pub indoor_doodads: std::collections::HashSet<u16>,
    /// The lamps of the building (the omni ones) and, for each doodad in a room, which of them shine there.
    pub lights: Vec<wowfmt::wmo::WmoLight>,
    pub doodad_lights: std::collections::HashMap<u16, Vec<u16>>,
    /// Liquid surfaces of all groups as a triangle list: (render-space position, liquid kind
    /// 0 water / 1 ocean / 2 magma / 3 slime), relative to the building.
    pub liquid_tris: Vec<([f32; 3], u32)>,
}

/// Two triangles per liquid cell that has liquid, in render space.
fn liquid_triangles(l: &WmoLiquid) -> Vec<([f32; 3], u32)> {
    let mut out = Vec::new();
    for cy in 0..l.y_verts - 1 {
        for cx in 0..l.x_verts - 1 {
            let flags = l.tiles[cy * (l.x_verts - 1) + cx];
            if flags & 0x0F == 0x0F {
                continue;
            }
            let corner = |x: usize, y: usize| {
                let h = l.heights[y * l.x_verts + x];
                [l.base[0] + x as f32 * LIQUID_CELL, h, -(l.base[1] + y as f32 * LIQUID_CELL)]
            };
            let (a, b, c, d) = (corner(cx, cy), corner(cx + 1, cy), corner(cx + 1, cy + 1), corner(cx, cy + 1));
            out.extend([a, d, b, b, d, c].map(|p| (p, (flags & 3) as u32)));
        }
    }
    out
}

impl WmoCpu {
    /// Doodads of the global set 0 plus the placement's own set.
    pub fn active_doodads(&self, set: u16) -> impl Iterator<Item = (usize, &WmoDoodad)> {
        let range = |i: usize| {
            let (first, count) = self.doodad_sets.get(i).copied().unwrap_or((0, 0));
            first as usize..(first + count) as usize
        };
        range(0).chain(if set == 0 { 0..0 } else { range(set as usize) }).filter_map(|i| self.doodads.get(i).map(|d| (i, d)))
    }
}

/// The lamp of a building as it lights one model: where it is (tile-local, render space), its colour times its intensity, and how far it reaches.
pub struct Lamp {
    pub position: Vec3,
    pub color: [f32; 3],
    pub attenuation: (f32, f32),
}

/// The model as it is lit by the building's lamps near it: its vertex colours carry the light of the (at most four) lamps nearest to it, which add to
/// what the room gives (`InteriorLight::baked`). One lamp lights a vertex by its colour times its intensity, the less the farther the vertex is (full
/// strength within the first length of the attenuation, none beyond the second) and the more the vertex faces it.
pub fn lit_by_lamps(model: &ModelCpu, matrix: &Mat4, lamps: &[Lamp]) -> Option<ModelCpu> {
    let centre = matrix.w_axis.truncate();
    let mut near: Vec<&Lamp> = lamps.iter().filter(|l| l.position.distance(centre) < l.attenuation.1 + 3.0).collect();
    near.sort_by(|a, b| a.position.distance(centre).total_cmp(&b.position.distance(centre)));
    near.truncate(4);
    if near.is_empty() {
        return None;
    }
    let vertices = model
        .vertices
        .iter()
        .map(|v| {
            let at = matrix.transform_point3(Vec3::from(v.pos));
            let normal = matrix.transform_vector3(Vec3::from(v.normal)).normalize_or_zero();
            let mut sum = [0.0f32; 3];
            for lamp in &near {
                let to = lamp.position - at;
                let d = to.length();
                let (start, end) = lamp.attenuation;
                let reach = if d >= end { 0.0 } else if d <= start { 1.0 } else { (end - d) / (end - start).max(1e-3) };
                let facing = if d < 1e-3 { 1.0 } else { normal.dot(to / d).max(0.0) };
                for (s, c) in sum.iter_mut().zip(lamp.color) {
                    *s += c * reach * facing;
                }
            }
            ModelVertex { color: [(sum[0].min(1.0) * 255.0) as u8, (sum[1].min(1.0) * 255.0) as u8, (sum[2].min(1.0) * 255.0) as u8, 0], ..*v }
        })
        .collect();
    Some(ModelCpu {
        key: format!("{}|lamps{:x}", model.key, (centre.x.to_bits() ^ centre.z.to_bits().rotate_left(7) ^ centre.y.to_bits().rotate_left(13))),
        vertices,
        indices: model.indices.clone(),
        batches: model.batches.clone(),
        col_vertices: model.col_vertices.clone(),
        col_indices: model.col_indices.clone(),
        tracks: model.tracks.clone(),
    })
}

/// Matrix of a WMO doodad relative to its building, in render space.
pub fn wmo_doodad_matrix(d: &WmoDoodad) -> Mat4 {
    // WMO model space (x east, y north, z up) -> render space (x east, y up, z south).
    let basis = Mat4::from_cols_array(&[1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.]);
    let local = Mat4::from_scale_rotation_translation(
        Vec3::splat(d.scale),
        glam::Quat::from_xyzw(d.rotation[0], d.rotation[1], d.rotation[2], d.rotation[3]).normalize(),
        Vec3::from(d.position),
    );
    basis * local * basis.transpose()
}

/// Placement files say `.mdx`/`.mdl`; the files in the archives are `.m2`.
pub fn normalize_model_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit_once('.') {
        Some((stem, "mdx" | "mdl")) => format!("{stem}.m2"),
        _ => lower,
    }
}

impl ModelCache {
    /// What the model `name` does if it has particle emitters of its own (a torch, a candelabra, a fountain) or moves (bones that animate, or billboards),
    /// which the placed model must show even when it has no mesh to draw.
    pub fn living(&self, mount: &Mount, name: &str) -> Option<Living> {
        let key = normalize_model_name(name);
        let (emits, moves) = *self.living.lock().unwrap().entry(key.clone()).or_insert_with(|| {
            let Some(m) = mount.read(&key).ok().and_then(|b| M2::parse(&b).ok()) else { return (false, false) };
            let animated = m.bones.iter().any(|b| b.rotation.keys.len() > 1 || b.translation.keys.len() > 1 || b.scale.keys.len() > 1);
            (!m.particles.is_empty(), animated || crate::anim::has_billboards(&m))
        });
        (emits || moves).then_some(Living { file: key, mesh: moves })
    }

    /// The sounds the model `name` plays all the time (a waterfall, a fountain): their places and `SoundEntries` numbers.
    pub fn sounds(&self, mount: &Mount, name: &str) -> Vec<(Vec3, u32)> {
        let key = normalize_model_name(name);
        self.sounds
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_insert_with(|| {
                let Some(m) = mount.read(&key).ok().and_then(|b| M2::parse(&b).ok()) else { return Vec::new() };
                m.events
                    .iter()
                    .filter(|e| &e.id == b"$DSL")
                    .map(|e| {
                        let pivot = m.bones.get(e.bone as usize).map_or([0.0; 3], |b| b.pivot);
                        let p = [pivot[0] + e.position[0], pivot[1] + e.position[1], pivot[2] + e.position[2]];
                        (Vec3::new(p[0], p[2], -p[1]), e.data)
                    })
                    .collect()
            })
            .clone()
    }

    pub fn get(&self, mount: &Mount, name: &str) -> Option<Arc<ModelCpu>> {
        let key = normalize_model_name(name);
        if let Some(hit) = self.models.lock().unwrap().get(&key) {
            return hit.clone();
        }
        let loaded = match self.load(mount, &key, &[]) {
            Ok(m) => Some(Arc::new(m)),
            Err(e) => {
                log::debug!("model {key}: {e:#}");
                None
            }
        };
        self.models.lock().unwrap().insert(key, loaded.clone());
        loaded
    }

    /// A weapon or shield model with its object-skin texture; the translucent shimmer layers are dropped.
    pub fn item(&self, mount: &Mount, model: &str, skin: &str) -> Result<Arc<ModelCpu>> {
        let mut m = self.load(mount, &model.to_ascii_lowercase(), &[(2, skin)])?;
        m.batches.retain(|b| b.blend != Blend::Additive);
        m.key = format!("item|{}|{}", model.to_ascii_lowercase(), skin.to_ascii_lowercase());
        Ok(Arc::new(m))
    }

    /// A small interface model (a quest mark) as it is shown: without the translucent glow layers, whose strength the
    /// model animates and which would wash the colour out to white.
    pub fn plain(&self, mount: &Mount, model: &str) -> Result<Arc<ModelCpu>> {
        let mut m = self.load(mount, &model.to_ascii_lowercase(), &[])?;
        m.batches.retain(|b| b.blend != Blend::Additive);
        m.key = format!("plain|{}", model.to_ascii_lowercase());
        Ok(Arc::new(m))
    }

    /// A creature model with its replaceable textures: `skins` are (texture type, file) pairs, e.g. the fur of a wolf.
    pub fn creature(&self, mount: &Mount, model: &str, skins: &[(u32, &str)]) -> Result<Arc<ModelCpu>> {
        let mut m = self.load(mount, &model.to_ascii_lowercase(), skins)?;
        m.key = format!("creature|{}|{}", model.to_ascii_lowercase(), skins.iter().map(|(_, s)| *s).collect::<Vec<_>>().join("|").to_ascii_lowercase());
        Ok(Arc::new(m))
    }

    /// All renderable groups of a WMO; empty if it cannot be loaded.
    pub fn get_wmo(&self, mount: &Mount, name: &str) -> Arc<WmoCpu> {
        let key = name.to_ascii_lowercase();
        if let Some(hit) = self.wmos.lock().unwrap().get(&key) {
            return hit.clone();
        }
        let wmo = match self.load_wmo(mount, name, &key) {
            Ok(w) => w,
            Err(e) => {
                log::debug!("wmo {key}: {e:#}");
                WmoCpu::default()
            }
        };
        let wmo = Arc::new(wmo);
        self.wmos.lock().unwrap().insert(key, wmo.clone());
        wmo
    }

    fn load_wmo(&self, mount: &Mount, path: &str, key: &str) -> Result<WmoCpu> {
        let root = WmoRoot::parse(&mount.read(path)?)?;
        let mut groups: Vec<WmoGroupCpu> = Vec::new();
        let mut liquid_tris = Vec::new();
        let mut indoor_doodads = std::collections::HashSet::new();
        let mut doodad_lights: std::collections::HashMap<u16, Vec<u16>> = std::collections::HashMap::new();
        let parsed: Vec<Option<WmoGroup>> = (0..root.group_count)
            .map(|g| {
                let gpath = group_path(path, g);
                mount
                    .read(&gpath)
                    .map_err(anyhow::Error::from)
                    .and_then(|d| Ok(WmoGroup::parse(&d)?))
                    .map_err(|e| log::debug!("wmo group {gpath}: {e:#}"))
                    .ok()
            })
            .collect();
        // The groups the outside belongs to (WoW.exe: `flags & 0x48`); the baked colours of rooms fade toward the light of the sun at their portals.
        let exterior: Vec<bool> = parsed.iter().map(|g| g.as_ref().is_some_and(|g| g.flags & 0x48 != 0)).collect();
        for group in parsed {
            let Some(group) = group else {
                groups.push(WmoGroupCpu::default());
                continue;
            };
            if let Some(l) = &group.liquid {
                liquid_tris.extend(liquid_triangles(l));
            }
            if group.indoor {
                indoor_doodads.extend(group.doodad_refs.iter().copied());
                for &d in &group.doodad_refs {
                    doodad_lights.entry(d).or_default().extend(group.light_refs.iter().copied());
                }
            }
            let slot = groups.len();
            groups.push(WmoGroupCpu {
                id: group.id,
                flags: group.flags,
                bbox: group.bbox,
                portals: (group.portal_start, group.portal_count),
                model: None,
                sunlit: group.tri_info.iter().map(|&(flags, _)| flags & 1 != 0).collect(),
            });
            if group.indices.is_empty() || group.batches.is_empty() {
                continue;
            }
            let baked = if group.indoor { group.faded_colors(&root, &exterior, MAP_OBJECT_OVERBRIGHT) } else { Vec::new() };
            let vertices = group
                .vertices
                .iter()
                .zip(&group.normals)
                .zip(&group.uvs)
                .enumerate()
                .map(|(i, ((p, n), uv))| ModelVertex {
                    pos: [p[0], p[2], -p[1]],
                    normal: [n[0], n[2], -n[1]],
                    uv: *uv,
                    color: baked.get(i).copied().unwrap_or([255, 255, 255, 0]),
                })
                .collect();
            let mut batches = Vec::new();
            for (bi, b) in group.batches.iter().enumerate() {
                if (b.start_index + b.index_count) as usize > group.indices.len() {
                    continue;
                }
                let mat = root.materials.get(b.material as usize);
                let texture = mat.filter(|m| !m.texture.is_empty()).and_then(|m| self.texture(mount, &m.texture));
                batches.push(BatchCpu {
                    index_start: b.start_index,
                    index_count: b.index_count,
                    texture,
                    wrap: true,
                    blend: match mat.map_or(0, |m| m.blend) {
                        0 => Blend::Opaque,
                        1 => Blend::AlphaKey,
                        3 => Blend::Additive,
                        _ => Blend::Alpha,
                    },
                    // The original draws the buildings with or without light by the kind of the batch alone; the material's flag 1 (unlit) is not read.
                    unlit: false,
                    baked: match group.batch_kind(bi) {
                        _ if baked.is_empty() => Baked::Exterior,
                        BatchKind::Interior => Baked::Interior,
                        BatchKind::Transition => Baked::Transition,
                        BatchKind::Exterior => Baked::Exterior,
                    },
                    window: mat.is_some_and(|m| m.flags & 0x20 != 0),
                    glow: mat.map_or([0; 3], |m| m.glow),
                    refs: Default::default(),
                    particle: false,
                });
            }
            if batches.is_empty() {
                continue;
            }
            batches.sort_by_key(|b| b.blend != Blend::Opaque && b.blend != Blend::AlphaKey);
            // Solid triangles: see `wowfmt::wmo::is_solid`.
            let col_vertices: Vec<[f32; 3]> = group.vertices.iter().map(|p| [p[0], p[2], -p[1]]).collect();
            let mut col_indices = Vec::new();
            for (t, tri) in group.indices.chunks_exact(3).enumerate() {
                if group.tri_info.get(t).is_some_and(|&(flags, _)| wowfmt::wmo::is_solid(flags)) {
                    col_indices.extend(tri.iter().map(|&i| i as u32));
                }
            }
            groups[slot].model = Some(Arc::new(ModelCpu {
                key: format!("{key}#{slot}"),
                vertices,
                indices: group.indices,
                batches,
                col_vertices,
                col_indices,
                tracks: None,
            }));
        }
        anyhow::ensure!(groups.iter().any(|g| g.model.is_some()), "no renderable groups");
        Ok(WmoCpu {
            id: root.id,
            groups,
            portal_verts: root.portal_verts,
            portals: root.portals,
            portal_refs: root.portal_refs,
            doodads: root.doodads,
            doodad_sets: root.doodad_sets,
            indoor_doodads,
            lights: root.lights,
            doodad_lights,
            liquid_tris,
        })
    }

    pub fn texture(&self, mount: &Mount, path: &str) -> Option<(String, Arc<RawTexture>)> {
        let key = path.to_ascii_lowercase();
        if let Some(hit) = self.textures.lock().unwrap().get(&key) {
            return hit.clone().map(|t| (key.clone(), t));
        }
        let decoded = mount
            .read(path)
            .map_err(anyhow::Error::from)
            .and_then(|d| Ok(Blp::parse(&d)?))
            .map(|blp| {
                Arc::new(RawTexture {
                    width: blp.width,
                    height: blp.height,
                    mips: blp.mips,
                })
            });
        let tex = match decoded {
            Ok(t) => Some(t),
            Err(e) => {
                log::debug!("model texture {path}: {e:#}");
                None
            }
        };
        self.textures.lock().unwrap().insert(key.clone(), tex.clone());
        tex.map(|t| (key, t))
    }

    fn load(&self, mount: &Mount, key: &str, skins: &[(u32, &str)]) -> Result<ModelCpu> {
        let m2 = M2::parse(&mount.read(key)?)?;
        // Model space (x east, y north, z up) -> render space (x east, y up, z south).
        let vertices: Vec<ModelVertex> = m2
            .vertices
            .iter()
            .map(|v| ModelVertex {
                pos: [v.pos[0], v.pos[2], -v.pos[1]],
                normal: [v.normal[0], v.normal[2], -v.normal[1]],
                uv: v.uv,
                color: [255, 255, 255, 0],
            })
            .collect();
        anyhow::ensure!(!m2.batches.is_empty(), "no renderable batches");

        let mut batches: Vec<_> = m2
            .batches
            .iter()
            .map(|b| {
                let tex = b.texture.map(|i| &m2.textures[i]);
                // Fixed-name textures load as they are; replaceable ones (the object skin of an item, the fur of a
                // creature) are the file `skins` gives for their type, and without one they stay untextured.
                let texture = tex
                    .and_then(|t| match t.kind {
                        0 if !t.name.is_empty() => Some(t.name.as_str()),
                        kind => skins.iter().find(|(k, _)| *k == kind).map(|(_, s)| *s),
                    })
                    .and_then(|name| self.texture(mount, name));
                BatchCpu {
                    index_start: b.index_start as u32,
                    index_count: b.index_count as u32,
                    texture,
                    wrap: tex.is_some_and(|t| t.flags & 3 != 0),
                    blend: b.blend,
                    unlit: b.unlit,
                    baked: Default::default(),
                    window: false,
                    glow: [0; 3],
                    refs: crate::anim::BatchRefs { color: b.color, transparency: b.transparency, uv: b.uv_anim },
                    particle: false,
                }
            })
            .collect();
        // Translucent geometry later (stable, so opaque batches keep their order).
        batches.sort_by_key(|b| b.blend != Blend::Opaque && b.blend != Blend::AlphaKey);
        // Collision mesh: model space (x east, y north, z up) -> render space.
        let col_vertices = m2.collision_vertices.iter().map(|v| [v[0], v[2], -v[1]]).collect();
        let col_indices = m2.collision_indices.iter().map(|&i| i as u32).collect();
        let tracks = crate::anim::LookTracks::of(&m2);
        Ok(ModelCpu { key: key.to_string(), vertices, indices: m2.indices, batches, col_vertices, col_indices, tracks })
    }
}

/// Transform of a placement in tile-local render space, following the client's convention
/// (heading about +Y offset by 90 degrees, then tilt about Z and X).
pub fn placement_matrix(local: Vec3, rotation_deg: [f32; 3], scale: f32) -> Mat4 {
    Mat4::from_translation(local)
        * Mat4::from_rotation_y((rotation_deg[1] - 90.0).to_radians())
        * Mat4::from_rotation_z((-rotation_deg[0]).to_radians())
        * Mat4::from_rotation_x(rotation_deg[2].to_radians())
        * Mat4::from_scale(Vec3::splat(if scale > 0.0 { scale } else { 1.0 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that read the real game data skip themselves when no install is found.
    fn mount() -> Option<Mount> {
        let dir = mpq::find_wow_dir()?;
        Mount::open_wow_dir(dir).ok()
    }

    #[test]
    fn placement_uses_the_clients_heading_convention() {
        // Heading 90 degrees cancels the client's -90 offset: no rotation, only translation + scale.
        let m = placement_matrix(Vec3::new(10.0, 5.0, 20.0), [0.0, 90.0, 0.0], 2.0);
        let p = m.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert!((p - Vec3::new(12.0, 5.0, 20.0)).length() < 1e-4, "{p}");
        // Heading 0 turns the model's +X axis towards +Z (south).
        let m = placement_matrix(Vec3::ZERO, [0.0, 0.0, 0.0], 1.0);
        let p = m.transform_point3(Vec3::X);
        assert!((p - Vec3::Z).length() < 1e-4, "{p}");
    }

    #[test]
    fn wmo_doodad_rotation_about_the_vertical_stays_vertical() {
        // 90 degrees about WMO z (up) is a rotation about render y (up) taking east to north (= -z).
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let d = WmoDoodad { name: String::new(), position: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, h, h], scale: 1.0, color: [255; 4] };
        let m = wmo_doodad_matrix(&d);
        assert!((m.transform_point3(Vec3::ZERO) - Vec3::new(1.0, 3.0, -2.0)).length() < 1e-4);
        assert!((m.transform_vector3(Vec3::X) - Vec3::NEG_Z).length() < 1e-4);
        assert!((m.transform_vector3(Vec3::Y) - Vec3::Y).length() < 1e-4);
    }

    #[test]
    fn wmo_liquid_cells_become_triangles_in_render_space() {
        let l = WmoLiquid {
            x_verts: 3,
            y_verts: 2,
            base: [10.0, 20.0, 0.0],
            material: 0,
            heights: vec![5.0; 6],
            // Left cell: magma. Right cell: no liquid.
            tiles: vec![0x46, 0x4F],
        };
        let t = liquid_triangles(&l);
        assert_eq!(t.len(), 6);
        assert!(t.iter().all(|&(p, kind)| p[1] == 5.0 && kind == 2));
        // Model y (north) becomes render -z.
        assert!(t.iter().any(|&(p, _)| p == [10.0, 5.0, -20.0]));
        assert!(t.iter().any(|&(p, _)| p == [10.0 + LIQUID_CELL, 5.0, -(20.0 + LIQUID_CELL)]));
    }

    #[test]
    fn model_names_map_mdx_to_m2() {
        assert_eq!(normalize_model_name(r"World\Foo\Bar.MDX"), r"world\foo\bar.m2");
        assert_eq!(normalize_model_name("a.mdl"), "a.m2");
        assert_eq!(normalize_model_name("a.m2"), "a.m2");
    }

    #[test]
    fn real_tree_has_an_alpha_keyed_two_sided_canopy() {
        let Some(mount) = mount() else { return eprintln!("skipped: no game data") };
        let cache = ModelCache::default();
        let log = cache.get(&mount, r"World\Azeroth\Elwynn\PassiveDoodads\Tree\ElwynnLog01.mdx").expect("model loads");
        assert!(log.batches.iter().any(|b| b.blend == Blend::AlphaKey && b.texture.is_some()));
        assert!(log.batches.iter().any(|b| b.blend == Blend::Opaque && b.texture.is_some()));
        for b in &log.batches {
            assert!((b.index_start + b.index_count) as usize <= log.indices.len());
        }
        assert!(log.indices.iter().all(|&i| (i as usize) < log.vertices.len()));
    }

    #[test]
    fn real_wmo_groups_are_self_consistent() {
        let Some(mount) = mount() else { return eprintln!("skipped: no game data") };
        let cache = ModelCache::default();
        let wmo = cache.get_wmo(&mount, r"World\wmo\Dungeon\MD_Goldmine\MD_Goldmine.wmo");
        let groups: Vec<_> = wmo.groups.iter().filter_map(|g| g.model.as_ref()).collect();
        assert!(groups.len() >= 5, "only {} groups", groups.len());
        for g in groups {
            assert!(g.indices.iter().all(|&i| (i as usize) < g.vertices.len()));
            for b in &g.batches {
                assert!((b.index_start + b.index_count) as usize <= g.indices.len());
                assert_eq!(b.index_count % 3, 0);
            }
        }
        // Cached: same allocation the second time.
        assert!(Arc::ptr_eq(&wmo, &cache.get_wmo(&mount, r"World\wmo\Dungeon\MD_Goldmine\MD_Goldmine.wmo")));
    }

    #[test]
    fn the_room_behind_the_door_of_the_goldshire_inn_fades_toward_the_outdoor_light() {
        let Some(mount) = mount() else { return eprintln!("skipped: no game data") };
        let path = r"World\wmo\Azeroth\Buildings\GoldshireInn\GoldshireInn.wmo";
        let root = WmoRoot::parse(&mount.read(path).unwrap()).unwrap();
        let groups: Vec<WmoGroup> = (0..root.group_count).map(|g| WmoGroup::parse(&mount.read(&group_path(path, g)).unwrap()).unwrap()).collect();
        let exterior: Vec<bool> = groups.iter().map(|g| g.flags & 0x48 != 0).collect();
        // Group 0 is the room behind the door: three transition batches, no interior ones.
        let hall = &groups[0];
        assert_eq!((hall.transition_batches, hall.interior_batches), (3, 0));
        let faded = hall.faded_colors(&root, &exterior, true);
        assert!(faded.iter().zip(&hall.colors).all(|(f, c)| f[..3].iter().zip(c).all(|(f, c)| f >= c)), "colours only move toward white");
        assert!(faded.iter().zip(&hall.colors).any(|(f, c)| f != c), "something fades");
        // What the renderer is given: the room behind the door is all transition batches, the main hall interior ones and one exterior one.
        let wmo = ModelCache::default().get_wmo(&mount, path);
        let modes = |g: usize| wmo.groups[g].model.as_ref().unwrap().batches.iter().map(|b| b.baked).collect::<Vec<_>>();
        assert!(modes(0).iter().all(|&m| m == Baked::Transition));
        assert_eq!(modes(5).iter().filter(|&&m| m == Baked::Interior).count(), 13);
        assert_eq!(modes(5).iter().filter(|&&m| m == Baked::Exterior).count(), 1);
        // The panes seen from inside are exterior batches of the rooms (lit by the sun as it is outside, from where it cannot shine on them), the
        // outer panes are the ones that glow at night, and nothing of a building is drawn "unlit".
        let batches: Vec<&BatchCpu> = wmo.groups.iter().flat_map(|g| g.model.iter()).flat_map(|m| &m.batches).collect();
        assert!(batches.iter().any(|b| b.window && b.baked == Baked::Exterior));
        assert!(batches.iter().any(|b| b.glow == [0xcb; 3] && b.baked == Baked::Exterior));
        assert!(batches.iter().all(|b| !b.unlit));
    }

    #[test]
    fn every_placed_model_of_northshire_loads() {
        let Some(mount) = mount() else { return eprintln!("skipped: no game data") };
        let adt = wowfmt::adt::Adt::parse(&mount.read(r"World\Maps\Azeroth\Azeroth_32_48.adt").unwrap()).unwrap();
        let cache = ModelCache::default();
        let missing: Vec<_> = adt
            .m2_names
            .iter()
            .filter(|n| cache.get(&mount, n).is_none())
            // Models that are legitimately absent (removed by the patch) or have no geometry.
            .filter(|n| mount.exists(&normalize_model_name(n)))
            .collect();
        assert!(missing.len() <= 3, "models that exist but failed to load: {missing:?}");
    }

    #[test]
    fn a_model_in_a_room_is_lit_by_its_places_colour_held_down_for_the_ambient_and_up_for_the_sun_direction() {
        // A candelabra of the abbey: blue 155, green 115, red 125.
        let InteriorLight { ambient, diffuse, .. } = interior_light([155, 115, 125, 255]);
        let top = |c: [f32; 3]| c.iter().cloned().fold(0.0, f32::max);
        assert!((top(ambient) - 96.0 / 255.0).abs() < 1e-4 && (top(diffuse) - 155.0 / 255.0).abs() < 1e-4);
        // A dark place gets a bright sun-direction light and keeps its own dark ambient.
        let InteriorLight { ambient, diffuse, .. } = interior_light([11, 11, 11, 255]);
        assert!((top(ambient) - 11.0 / 255.0).abs() < 1e-4 && (top(diffuse) - 112.0 / 255.0).abs() < 1e-3);
    }
}
