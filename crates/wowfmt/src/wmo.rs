//! WMO world objects (buildings, caves), version 17.
//!
//! A WMO is a root file (`name.wmo`: materials, group list) plus one file per group
//! (`name_000.wmo`, ...) holding the renderable geometry. Model space: x east,
//! y north, z up (same as M2).

use crate::reader::{Chunks, Reader};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct WmoMaterial {
    pub flags: u32,
    /// 0 opaque, 1 alpha key, otherwise blended.
    pub blend: u32,
    pub texture: String,
}

/// A model placed inside the building (`MODD`), in WMO model space.
#[derive(Clone, Debug)]
pub struct WmoDoodad {
    pub name: String,
    pub position: [f32; 3],
    /// Quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub scale: f32,
    /// The light of the place it stands in (`MODD`'s colour, blue, green, red, alpha): what it is lit by indoors.
    pub color: [u8; 4],
}

/// A light of the building (`MOLT`): a point of light that lights the models near it.
#[derive(Clone, Copy, Debug)]
pub struct WmoLight {
    /// 0 for the omni lights, the only ones the original makes.
    pub kind: u8,
    /// Blue, green, red, alpha as the file has it.
    pub color: [u8; 4],
    pub position: [f32; 3],
    pub intensity: f32,
    /// How far from it it is at full strength, and how far it reaches.
    pub attenuation: (f32, f32),
}

/// A portal polygon (`MOPT`): `count` vertices from `MOPV` starting at `start`, and its plane.
#[derive(Clone, Copy, Debug)]
pub struct WmoPortal {
    pub start: u16,
    pub count: u16,
    /// Normal and distance: `n . p + d = 0`.
    pub plane: [f32; 4],
}

/// A group's reference to a portal (`MOPR`).
#[derive(Clone, Copy, Debug)]
pub struct PortalRef {
    pub portal: u16,
    /// The group on the other side (0xFFFF = none).
    pub group: u16,
    /// Negative: the group lies on the negative side of the plane.
    pub side: i16,
}

pub struct WmoRoot {
    /// The building's number in `WMOAreaTable` (`MOHD`).
    pub id: u32,
    pub group_count: usize,
    pub materials: Vec<WmoMaterial>,
    pub doodads: Vec<WmoDoodad>,
    /// `MODS`: (first doodad, count). Set 0 is the global set, always shown.
    pub doodad_sets: Vec<(u32, u32)>,
    pub lights: Vec<WmoLight>,
    pub portal_verts: Vec<[f32; 3]>,
    pub portals: Vec<WmoPortal>,
    pub portal_refs: Vec<PortalRef>,
}

#[derive(Clone, Copy, Debug)]
pub struct WmoBatch {
    pub start_index: u32,
    pub index_count: u32,
    pub material: u8,
}

/// A liquid surface of a group (`MLIQ`): a grid of heights over the model-space xy plane.
pub struct WmoLiquid {
    pub x_verts: usize,
    pub y_verts: usize,
    /// Model-space corner of vertex (0, 0); vertices are `LIQUID_CELL` apart.
    pub base: [f32; 3],
    pub material: u16,
    /// `y_verts * x_verts` heights, x fastest.
    pub heights: Vec<f32>,
    /// `(y_verts - 1) * (x_verts - 1)` cell flags; low nibble 0x0F = no liquid in the cell.
    pub tiles: Vec<u8>,
}

/// Edge length of one liquid cell in yards.
pub const LIQUID_CELL: f32 = 533.333_3 / 128.0;

pub struct WmoGroup {
    /// The group's number in `WMOAreaTable`.
    pub id: u32,
    pub flags: u32,
    /// Model-space bounds from the MOGP header.
    pub bbox: ([f32; 3], [f32; 3]),
    pub portal_start: u16,
    pub portal_count: u16,
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    /// Per triangle: (flags, material id). Material 0xFF = not rendered (collision only).
    pub tri_info: Vec<(u8, u8)>,
    pub batches: Vec<WmoBatch>,
    /// Baked lighting (`MOCV`) as RGBA; empty if the group has none.
    pub colors: Vec<[u8; 4]>,
    /// Interior group: lit only by its baked vertex colours, not by the sun.
    pub indoor: bool,
    /// The doodads of the building that stand in this group (`MODR`: indices into the building's doodads).
    pub doodad_refs: Vec<u16>,
    /// The lights of the building that shine in this group (`MOLR`: indices into the building's lights).
    pub light_refs: Vec<u16>,
    pub liquid: Option<WmoLiquid>,
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

impl WmoRoot {
    pub fn parse(data: &[u8]) -> Result<WmoRoot> {
        let mut group_count = None;
        let mut id = 0;
        let mut texture_blob: &[u8] = &[];
        let mut raw_materials: Vec<(u32, u32, u32)> = Vec::new();
        let mut doodad_blob: &[u8] = &[];
        let mut raw_doodads: &[u8] = &[];
        let mut doodad_sets = Vec::new();
        let mut lights = Vec::new();
        let mut portal_verts = Vec::new();
        let mut portals = Vec::new();
        let mut portal_refs = Vec::new();
        for c in Chunks::new(data) {
            let c = c?;
            match &c.tag {
                b"MVER" => {
                    let v = Reader::new(c.data).u32()?;
                    if v != 17 {
                        return Err(Error::Unsupported(format!("WMO version {v}, expected 17")));
                    }
                }
                b"MOHD" => {
                    let mut r = Reader::new(c.data);
                    let _materials = r.u32()?;
                    group_count = Some(r.u32()? as usize);
                    // Portals, lights, models, doodads, sets and the ambient colour come before the building's number.
                    for _ in 0..6 {
                        r.u32()?;
                    }
                    id = r.u32()?;
                }
                b"MOTX" => texture_blob = c.data,
                b"MOMT" => {
                    for m in c.data.chunks_exact(64) {
                        raw_materials.push((le32(&m[0..]), le32(&m[8..]), le32(&m[12..])));
                    }
                }
                b"MOPV" => portal_verts = c.data.chunks_exact(12).map(vec3).collect(),
                b"MOPT" => {
                    portals = c
                        .data
                        .chunks_exact(20)
                        .map(|p| WmoPortal {
                            start: u16::from_le_bytes([p[0], p[1]]),
                            count: u16::from_le_bytes([p[2], p[3]]),
                            plane: [4, 8, 12, 16].map(|i| f32::from_bits(le32(&p[i..]))),
                        })
                        .collect()
                }
                b"MOPR" => {
                    portal_refs = c
                        .data
                        .chunks_exact(8)
                        .map(|e| PortalRef {
                            portal: u16::from_le_bytes([e[0], e[1]]),
                            group: u16::from_le_bytes([e[2], e[3]]),
                            side: i16::from_le_bytes([e[4], e[5]]),
                        })
                        .collect()
                }
                b"MODN" => doodad_blob = c.data,
                b"MODD" => raw_doodads = c.data,
                b"MOLT" => {
                    lights = c
                        .data
                        .chunks_exact(48)
                        .map(|l| {
                            let f = |i: usize| f32::from_bits(le32(&l[i..]));
                            WmoLight { kind: l[0], color: [l[4], l[5], l[6], l[7]], position: [f(8), f(12), f(16)], intensity: f(20), attenuation: (f(40), f(44)) }
                        })
                        .collect()
                }
                b"MODS" => doodad_sets = c.data.chunks_exact(32).map(|s| (le32(&s[20..]), le32(&s[24..]))).collect(),
                _ => {}
            }
        }
        let group_count = group_count.ok_or_else(|| Error::Invalid("WMO has no MOHD".into()))?;
        let materials = raw_materials
            .into_iter()
            .map(|(flags, blend, ofs)| WmoMaterial { flags, blend, texture: cstr(texture_blob, ofs) })
            .collect();
        let doodads = raw_doodads
            .chunks_exact(40)
            .map(|d| {
                let f = |i: usize| f32::from_bits(le32(&d[i..]));
                WmoDoodad {
                    name: cstr(doodad_blob, le32(d) & 0xFF_FFFF),
                    position: [f(4), f(8), f(12)],
                    rotation: [f(16), f(20), f(24), f(28)],
                    scale: f(32),
                    color: [d[36], d[37], d[38], d[39]],
                }
            })
            .collect();
        Ok(WmoRoot { id, group_count, materials, doodads, doodad_sets, lights, portal_verts, portals, portal_refs })
    }
}

fn cstr(blob: &[u8], offset: u32) -> String {
    let s = blob.get(offset as usize..).unwrap_or(&[]);
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).into_owned()
}

/// MOPY triangle flags.
pub const TRI_DETAIL: u8 = 0x04;
pub const TRI_COLLISION: u8 = 0x08;
pub const TRI_HINT: u8 = 0x10;
pub const TRI_RENDER: u8 = 0x20;

/// Whether a triangle blocks the player: rendered faces, detail faces and invisible
/// collision-only faces do; BSP hint triangles do not.
///
/// Derived from the data: in the Abbey every rendered triangle carries `0x20` (some also
/// `0x04`, `0x40`), invisible helper geometry carries `0x08`, and `0x10` never occurs on
/// either. An earlier rule that required `0x04` left most interior floors non-solid.
pub fn is_solid(flags: u8) -> bool {
    flags & (TRI_RENDER | TRI_DETAIL | TRI_COLLISION) != 0
}

/// Group file name for group `index` of the WMO at `root_path`.
pub fn group_path(root_path: &str, index: usize) -> String {
    let stem = root_path.strip_suffix(".wmo").or_else(|| root_path.strip_suffix(".WMO")).unwrap_or(root_path);
    format!("{stem}_{index:03}.wmo")
}

impl WmoGroup {
    pub fn parse(data: &[u8]) -> Result<WmoGroup> {
        let mogp = Chunks::new(data)
            .find_map(|c| match c {
                Ok(c) if &c.tag == b"MOGP" => Some(Ok(c)),
                Err(e) => Some(Err(e)),
                _ => None,
            })
            .ok_or_else(|| Error::Invalid("WMO group has no MOGP".into()))??;
        if mogp.data.len() < 68 {
            return Err(Error::Invalid("MOGP header too small".into()));
        }
        let flags = le32(&mogp.data[8..]);
        let id = le32(&mogp.data[56..]);
        let bbox = (vec3(&mogp.data[12..]), vec3(&mogp.data[24..]));
        let portal_start = u16::from_le_bytes([mogp.data[36], mogp.data[37]]);
        let portal_count = u16::from_le_bytes([mogp.data[38], mogp.data[39]]);

        let (mut vertices, mut normals, mut uvs, mut indices, mut batches) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut tri_info = Vec::new();
        let mut colors = Vec::new();
        let mut liquid = None;
        let mut doodad_refs = Vec::new();
        let mut light_refs = Vec::new();
        for c in Chunks::new(&mogp.data[68..]) {
            let Ok(c) = c else { break };
            match &c.tag {
                b"MOVT" => vertices = c.data.chunks_exact(12).map(|v| vec3(v)).collect(),
                b"MONR" => normals = c.data.chunks_exact(12).map(|v| vec3(v)).collect(),
                b"MOTV" if uvs.is_empty() => {
                    uvs = c
                        .data
                        .chunks_exact(8)
                        .map(|v| [f32::from_bits(le32(&v[0..])), f32::from_bits(le32(&v[4..]))])
                        .collect()
                }
                // Stored BGRA.
                b"MOCV" => colors = c.data.chunks_exact(4).map(|v| [v[2], v[1], v[0], v[3]]).collect(),
                b"MLIQ" => liquid = WmoLiquid::parse(c.data),
                b"MOLR" => light_refs = c.data.chunks_exact(2).map(|v| u16::from_le_bytes([v[0], v[1]])).collect(),
                b"MODR" => doodad_refs = c.data.chunks_exact(2).map(|v| u16::from_le_bytes([v[0], v[1]])).collect(),
                b"MOPY" => tri_info = c.data.chunks_exact(2).map(|p| (p[0], p[1])).collect(),
                b"MOVI" => indices = c.data.chunks_exact(2).map(|v| u16::from_le_bytes([v[0], v[1]])).collect(),
                b"MOBA" => {
                    // 6 x i16 bounds, u32 start index, u16 count, u16 min, u16 max, u8 flags, u8 material
                    batches = c
                        .data
                        .chunks_exact(24)
                        .map(|b| WmoBatch {
                            start_index: le32(&b[12..]),
                            index_count: u16::from_le_bytes([b[16], b[17]]) as u32,
                            material: b[23],
                        })
                        .collect()
                }
                _ => {}
            }
        }
        if let Some(&max) = indices.iter().max() {
            if max as usize >= vertices.len() {
                return Err(Error::Invalid("WMO index outside vertex array".into()));
            }
        }
        normals.resize(vertices.len(), [0.0, 0.0, 1.0]);
        uvs.resize(vertices.len(), [0.0, 0.0]);
        if !colors.is_empty() {
            colors.resize(vertices.len(), [255; 4]);
        }
        Ok(WmoGroup { id, flags, bbox, portal_start, portal_count, vertices, normals, uvs, indices, tri_info, batches, colors, indoor: flags & 0x2000 != 0, doodad_refs, light_refs, liquid })
    }
}

impl WmoLiquid {
    fn parse(d: &[u8]) -> Option<WmoLiquid> {
        let mut r = Reader::new(d);
        let (xv, yv) = (r.u32().ok()? as usize, r.u32().ok()? as usize);
        let (xt, yt) = (r.u32().ok()? as usize, r.u32().ok()? as usize);
        let base = r.vec3().ok()?;
        let material = r.u16().ok()?;
        // Each vertex is 4 bytes of flow / texture data followed by the height.
        let verts = d.get(30..30 + xv * yv * 8)?;
        let tiles = d.get(30 + xv * yv * 8..30 + xv * yv * 8 + xt * yt)?;
        (xv == xt + 1 && yv == yt + 1).then(|| WmoLiquid {
            x_verts: xv,
            y_verts: yv,
            base,
            material,
            heights: verts.chunks_exact(8).map(|v| f32::from_bits(le32(&v[4..]))).collect(),
            tiles: tiles.to_vec(),
        })
    }
}

fn vec3(v: &[u8]) -> [f32; 3] {
    [f32::from_bits(le32(&v[0..])), f32::from_bits(le32(&v[4..])), f32::from_bits(le32(&v[8..]))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solidity_of_the_flag_combinations_seen_in_the_data() {
        // Rendered faces, with and without the detail / 0x40 bits.
        for f in [0x20, 0x24, 0x60, 0x64, 0x21, 0x65] {
            assert!(is_solid(f), "{f:#x}");
        }
        // Invisible collision geometry.
        for f in [0x08, 0x48, 0x09] {
            assert!(is_solid(f), "{f:#x}");
        }
        // Hints and nothing at all do not collide.
        assert!(!is_solid(TRI_HINT));
        assert!(!is_solid(0));
    }
}
