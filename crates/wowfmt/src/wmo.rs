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
    /// The colour the material glows with at night (`MOMT` flag 0x10, the SIDN colour), red, green, blue; black for the others.
    pub glow: [u8; 3],
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
    /// Highest vertex the batch uses.
    pub last_vertex: u16,
    pub material: u8,
}

/// What a batch is lit by (the batch counts of the `MOGP` header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchKind {
    /// The baked colours, made brighter by their alpha (the overbright pixel shader).
    Interior,
    /// The sun, with the baked colours as the colour of the surface.
    Exterior,
    /// The baked colours and the sun, mixed by the alpha of the baked colour.
    Transition,
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
    /// How many of the first batches are transition batches, and how many interior ones follow; the rest are exterior.
    pub transition_batches: u16,
    pub interior_batches: u16,
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
        let mut raw_materials: Vec<(u32, u32, u32, [u8; 3])> = Vec::new();
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
                        // Flags, blend mode, texture name offset, and the SIDN colour (blue, green, red, alpha).
                        let glow = if le32(&m[0..]) & 0x10 != 0 { [m[18], m[17], m[16]] } else { [0; 3] };
                        raw_materials.push((le32(&m[0..]), le32(&m[8..]), le32(&m[12..]), glow));
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
            .map(|(flags, blend, ofs, glow)| WmoMaterial { flags, blend, texture: cstr(texture_blob, ofs), glow })
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
        let transition_batches = u16::from_le_bytes([mogp.data[40], mogp.data[41]]);
        let interior_batches = u16::from_le_bytes([mogp.data[42], mogp.data[43]]);

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
                            last_vertex: u16::from_le_bytes([b[20], b[21]]),
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
        Ok(WmoGroup { id, flags, bbox, portal_start, portal_count, transition_batches, interior_batches, vertices, normals, uvs, indices, tri_info, batches, colors, indoor: flags & 0x2000 != 0, doodad_refs, light_refs, liquid })
    }

    /// What lights batch number `batch`: the batches are in the order transition, interior, exterior.
    pub fn batch_kind(&self, batch: usize) -> BatchKind {
        let transition = self.transition_batches as usize;
        if batch < transition {
            BatchKind::Transition
        } else if batch < transition + self.interior_batches as usize {
            BatchKind::Interior
        } else {
            BatchKind::Exterior
        }
    }

    /// The baked colours as WoW.exe leaves them for drawing. `exterior` tells per group of the building whether it is one that the
    /// outside belongs to (`flags & 0x48`); `overbright` is whether the pixel shader that reads the alpha of the colours is in use.
    ///
    /// Without it (`FUN_006c41e0`), the vertices of the triangles that lack MOPY flag 1 get alpha 255 first, which keeps them from
    /// fading below. Then (`FUN_006c43d0`) a vertex of the transition batches that lies on a portal to the outside becomes white and
    /// fully outdoor lit; one within `FADE_RANGE` of such a portal moves toward that, the more the closer, unless it has an alpha.
    pub fn faded_colors(&self, root: &WmoRoot, exterior: &[bool], overbright: bool) -> Vec<[u8; 4]> {
        let mut colors = self.colors.clone();
        if !overbright {
            for (tri, &(flags, _)) in self.indices.chunks_exact(3).zip(&self.tri_info) {
                if flags & 1 == 0 {
                    tri.iter().for_each(|&i| colors[i as usize][3] = 255);
                }
            }
        }
        let Some(last) = (self.transition_batches as usize).checked_sub(1).and_then(|b| self.batches.get(b)) else { return colors };
        let (first, count) = (self.portal_start as usize, self.portal_count as usize);
        let portals: Vec<&WmoPortal> = root
            .portal_refs
            .get(first..first + count)
            .unwrap_or(&[])
            .iter()
            .filter(|r| exterior.get(r.group as usize) == Some(&true))
            .filter_map(|r| root.portals.get(r.portal as usize))
            .collect();
        for (c, &p) in colors.iter_mut().zip(&self.vertices).take(last.last_vertex as usize + 1) {
            let d = portals.iter().map(|portal| root.portal_distance(portal, p)).fold(f32::MAX, f32::min);
            if d == 0.0 {
                *c = [255; 4];
            } else if d < FADE_RANGE && c[3] == 0 {
                let f = 1.0 - d * 0.15;
                let toward_white = |c: u8| (c as f32 + (255.0 - c as f32) * f) as u8;
                *c = [toward_white(c[0]), toward_white(c[1]), toward_white(c[2]), (f * 255.0) as u8];
            }
        }
        colors
    }
}

/// How far from a portal to the outside the baked colours start to fade toward the outdoor light, in yards.
pub const FADE_RANGE: f32 = 6.666_667;

impl WmoRoot {
    /// How far `p` is from the polygon of a portal, as `FUN_006c4240` measures it: 0 on the polygon (within a sixth of a yard of
    /// its plane and inside its outline), else the distance to the nearest edge.
    pub fn portal_distance(&self, portal: &WmoPortal, p: [f32; 3]) -> f32 {
        let (start, count) = (portal.start as usize, portal.count as usize);
        let Some(poly) = self.portal_verts.get(start..start + count).filter(|v| !v.is_empty()) else { return f32::MAX };
        let n = [portal.plane[0], portal.plane[1], portal.plane[2]];
        let off = n[0] * p[0] + n[1] * p[1] + n[2] * p[2] + portal.plane[3];
        if off * off < 1.0 / 36.0 && inside_polygon(poly, n, p) {
            return 0.0;
        }
        (0..count).map(|i| segment_distance(poly[(i + count - 1) % count], poly[i], p)).fold(f32::MAX, f32::min)
    }
}

/// Whether `p` projects inside the polygon along its normal's strongest axis (crossing test).
fn inside_polygon(poly: &[[f32; 3]], n: [f32; 3], p: [f32; 3]) -> bool {
    let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());
    let drop = if ax <= ay { if az < ay { 1 } else { 2 } } else if az < ax { 0 } else { 2 };
    let (u, v) = match drop { 0 => (1, 2), 1 => (0, 2), _ => (0, 1) };
    let mut inside = false;
    let mut prev = poly[poly.len() - 1];
    for &cur in poly {
        if (prev[v] <= p[v]) != (cur[v] <= p[v]) && p[u] < prev[u] + (p[v] - prev[v]) / (cur[v] - prev[v]) * (cur[u] - prev[u]) {
            inside = !inside;
        }
        prev = cur;
    }
    inside
}

fn segment_distance(a: [f32; 3], b: [f32; 3], p: [f32; 3]) -> f32 {
    let (ab, ap) = ([0, 1, 2].map(|i| b[i] - a[i]), [0, 1, 2].map(|i| p[i] - a[i]));
    let len2: f32 = ab.iter().map(|c| c * c).sum();
    let t = if len2 > 0.0 { (ab.iter().zip(&ap).map(|(x, y)| x * y).sum::<f32>() / len2).clamp(0.0, 1.0) } else { 0.0 };
    (0..3).map(|i| (ap[i] - ab[i] * t).powi(2)).sum::<f32>().sqrt()
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

    /// A door of 2 x 2 yards in the plane x = 0, and a group with one transition batch of four vertices.
    fn door() -> (WmoRoot, WmoGroup) {
        let root = WmoRoot {
            id: 0,
            group_count: 2,
            materials: Vec::new(),
            doodads: Vec::new(),
            doodad_sets: Vec::new(),
            lights: Vec::new(),
            portal_verts: vec![[0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 2.0], [0.0, -1.0, 2.0]],
            portals: vec![WmoPortal { start: 0, count: 4, plane: [1.0, 0.0, 0.0, 0.0] }],
            portal_refs: vec![PortalRef { portal: 0, group: 1, side: 1 }],
        };
        let group = WmoGroup {
            id: 0,
            flags: 0x2005,
            bbox: ([0.0; 3], [0.0; 3]),
            portal_start: 0,
            portal_count: 1,
            transition_batches: 1,
            interior_batches: 0,
            // On the door, 1 yard above its edge, 3 yards beside its edge, and beyond the fade.
            vertices: vec![[0.0, 0.0, 1.0], [0.0, 0.0, 3.0], [0.0, 4.0, 1.0], [10.0, 0.0, 1.0]],
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: vec![0, 1, 2, 1, 2, 3],
            tri_info: Vec::new(),
            batches: vec![WmoBatch { start_index: 0, index_count: 6, last_vertex: 3, material: 0 }],
            colors: vec![[40, 30, 20, 0]; 4],
            indoor: true,
            doodad_refs: Vec::new(),
            light_refs: Vec::new(),
            liquid: None,
        };
        (root, group)
    }

    #[test]
    fn baked_colours_fade_toward_white_at_a_portal_to_the_outside() {
        let (root, group) = door();
        let colors = group.faded_colors(&root, &[false, true], true);
        // On the door: white, fully outdoor lit.
        assert_eq!(colors[0], [255; 4]);
        // 1 yard from the door's edge: f = 0.85.
        assert_eq!(colors[1],[40 + ((215.0 * 0.85) as u8), 30 + ((225.0 * 0.85) as u8), 20 + ((235.0 * 0.85) as u8), (0.85 * 255.0) as u8]);
        // 3 yards from the door's edge: f = 0.55.
        assert_eq!(colors[2][3], (0.55f32 * 255.0) as u8);
        // Past the range, and with a group that is not the outside: unchanged.
        assert_eq!(colors[3], [40, 30, 20, 0]);
        assert_eq!(group.faded_colors(&root, &[false, false], true), group.colors);
    }

    #[test]
    fn an_alpha_the_file_gave_is_kept() {
        let (root, mut group) = door();
        group.colors[1][3] = 90;
        assert_eq!(group.faded_colors(&root, &[false, true], true)[1], [40, 30, 20, 90]);
    }

    #[test]
    fn without_the_overbright_shader_triangles_without_flag_1_are_opaque_and_do_not_fade() {
        let (root, mut group) = door();
        // Triangle 0 (vertices 0, 1, 2) has flag 1, triangle 1 (1, 2, 3) does not: vertices 1, 2 and 3 get alpha 255.
        group.tri_info = vec![(1, 0), (0, 0)];
        let colors = group.faded_colors(&root, &[false, true], false);
        assert_eq!(colors[0], [255; 4], "on the door, still white");
        assert_eq!([colors[1], colors[2], colors[3]], [[40, 30, 20, 255]; 3]);
    }

    #[test]
    fn batches_come_in_the_order_transition_interior_exterior() {
        let (_, mut group) = door();
        group.transition_batches = 2;
        group.interior_batches = 1;
        let kinds: Vec<_> = (0..5).map(|b| group.batch_kind(b)).collect();
        assert_eq!(kinds, [BatchKind::Transition, BatchKind::Transition, BatchKind::Interior, BatchKind::Exterior, BatchKind::Exterior]);
    }
}
