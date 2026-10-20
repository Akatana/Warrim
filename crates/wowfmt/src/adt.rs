//! ADT terrain tiles (version 18, single-file layout used by 1.12).
//!
//! A tile is 16x16 map chunks (MCNK). Each chunk has a 9x9 + 8x8 interleaved
//! height grid, up to 4 texture layers blended with 64x64 4-bit alpha maps,
//! and (in this era) an inline MCLQ liquid block.

use crate::reader::{Chunk, Chunks, Reader};
use crate::{Error, Result};

pub const TILE_SIZE: f32 = 533.333_3;
pub const CHUNK_SIZE: f32 = TILE_SIZE / 16.0;
pub const UNIT_SIZE: f32 = CHUNK_SIZE / 8.0;
pub const CHUNKS_PER_SIDE: usize = 16;
pub const HEIGHTS_PER_CHUNK: usize = 145;
pub const ALPHA_DIM: usize = 64;

pub struct Adt {
    pub textures: Vec<String>,
    pub m2_names: Vec<String>,
    pub wmo_names: Vec<String>,
    pub doodads: Vec<DoodadPlacement>,
    pub wmos: Vec<WmoPlacement>,
    /// 256 entries, `index = chunk_y * 16 + chunk_x`.
    pub chunks: Vec<MapChunk>,
}

#[derive(Clone, Debug)]
pub struct DoodadPlacement {
    pub name_index: usize,
    pub unique_id: u32,
    /// As stored: (east, height, south) in yards from the map's north-west corner, i.e.
    /// render space shifted by 17066.67 on x and z. Tile `(a, b)` holds x in `a*533.33..`.
    pub position: [f32; 3],
    /// Degrees; file order (x, y = heading, z).
    pub rotation: [f32; 3],
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub struct WmoPlacement {
    pub name_index: usize,
    pub unique_id: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub flags: u16,
    pub doodad_set: u16,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub texture: u32,
    pub flags: u32,
    /// 64x64 blend weights (0..=255). Layer 0 has none (fully opaque base).
    pub alpha: Option<Box<[u8; ALPHA_DIM * ALPHA_DIM]>>,
}

#[derive(Clone, Debug)]
pub struct Liquid {
    pub kind: u32,
    pub min_height: f32,
    pub max_height: f32,
    /// 9x9 heights (row-major).
    pub heights: [f32; 81],
    /// The first two bytes of each vertex's leading word: how deep the liquid is there, and how it flows.
    pub depths: [u8; 81],
    pub flows: [u8; 81],
    /// 8x8 per-cell flags; 0x0F = no liquid in this cell.
    pub cell_flags: [u8; 64],
}

#[derive(Clone, Debug)]
pub struct MapChunk {
    pub index_x: u32,
    pub index_y: u32,
    pub flags: u32,
    pub area_id: u32,
    /// 4x4 bit-grid of 2x2 cell holes.
    pub holes: u32,
    /// World position of the chunk origin (x = north, y = west, z = base height).
    pub position: [f32; 3],
    /// Absolute heights (base + stored offset), in the interleaved 9-8-9-8... order.
    pub heights: [f32; HEIGHTS_PER_CHUNK],
    pub layers: Vec<Layer>,
    pub liquid: Option<Liquid>,
}

impl MapChunk {
    pub fn is_hole(&self, cell_x: usize, cell_y: usize) -> bool {
        self.holes & (1 << ((cell_y / 2) * 4 + cell_x / 2)) != 0
    }
}

fn cstrings(data: &[u8]) -> Vec<String> {
    data.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

impl Adt {
    pub fn parse(data: &[u8]) -> Result<Adt> {
        let mut textures = Vec::new();
        let mut m2_names = Vec::new();
        let mut wmo_names = Vec::new();
        let mut m2_offsets: Vec<u32> = Vec::new();
        let mut wmo_offsets: Vec<u32> = Vec::new();
        let mut m2_blob: &[u8] = &[];
        let mut wmo_blob: &[u8] = &[];
        let mut doodads_raw = Vec::new();
        let mut wmos_raw = Vec::new();
        let mut chunks = Vec::with_capacity(256);

        for c in Chunks::new(data) {
            let c = c?;
            match &c.tag {
                b"MVER" => {
                    let v = Reader::new(c.data).u32()?;
                    if v != 18 {
                        return Err(Error::Invalid(format!("ADT version {v}, expected 18")));
                    }
                }
                b"MTEX" => textures = cstrings(c.data),
                b"MMDX" => {
                    m2_blob = c.data;
                    m2_names = cstrings(c.data);
                }
                b"MMID" => m2_offsets = c.data.chunks_exact(4).map(le32).collect(),
                b"MWMO" => {
                    wmo_blob = c.data;
                    wmo_names = cstrings(c.data);
                }
                b"MWID" => wmo_offsets = c.data.chunks_exact(4).map(le32).collect(),
                b"MDDF" => {
                    for e in c.data.chunks_exact(36) {
                        let mut r = Reader::new(e);
                        doodads_raw.push((r.u32()?, r.u32()?, r.vec3()?, r.vec3()?, r.u16()?));
                    }
                }
                b"MODF" => {
                    for e in c.data.chunks_exact(64) {
                        let mut r = Reader::new(e);
                        let (id, uid, pos, rot) = (r.u32()?, r.u32()?, r.vec3()?, r.vec3()?);
                        r.skip(24)?; // bounding box
                        wmos_raw.push((id, uid, pos, rot, r.u16()?, r.u16()?));
                    }
                }
                b"MCNK" => chunks.push(parse_mcnk(&c)?),
                _ => {}
            }
        }
        if chunks.len() != 256 {
            return Err(Error::Invalid(format!("ADT has {} MCNK chunks, expected 256", chunks.len())));
        }

        // MMID/MWID hold byte offsets into the name blobs; map them to name indices.
        let name_index = |blob: &[u8], offsets: &[u32], entry: u32| -> usize {
            let Some(&off) = offsets.get(entry as usize) else { return usize::MAX };
            let prefix = &blob[..(off as usize).min(blob.len())];
            prefix.split(|&b| b == 0).filter(|s| !s.is_empty()).count()
        };
        let doodads = doodads_raw
            .into_iter()
            .map(|(id, unique_id, position, rotation, scale)| DoodadPlacement {
                name_index: name_index(m2_blob, &m2_offsets, id),
                unique_id,
                position,
                rotation,
                scale: scale as f32 / 1024.0,
            })
            .collect();
        let wmos = wmos_raw
            .into_iter()
            .map(|(id, unique_id, position, rotation, flags, doodad_set)| WmoPlacement {
                name_index: name_index(wmo_blob, &wmo_offsets, id),
                unique_id,
                position,
                rotation,
                flags,
                doodad_set,
            })
            .collect();

        Ok(Adt { textures, m2_names, wmo_names, doodads, wmos, chunks })
    }
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Sub-chunk located through an offset in the MCNK header (offsets are relative
/// to the start of the MCNK chunk header, i.e. 8 bytes before its payload).
fn subchunk<'a>(mcnk: &Chunk<'a>, rel: u32, tag: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    if rel == 0 {
        return Ok(None);
    }
    let start = (rel as usize).checked_sub(8).ok_or_else(|| Error::Invalid("bad MCNK offset".into()))?;
    let Some(rest) = mcnk.data.get(start..) else {
        return Err(Error::Invalid("MCNK sub-chunk offset past end".into()));
    };
    let mut it = Chunks::new(rest);
    match it.next() {
        Some(Ok(c)) if &c.tag == tag => Ok(Some(c.data)),
        Some(Ok(c)) => Err(Error::Invalid(format!(
            "expected {} at MCNK+{rel}, found {}",
            String::from_utf8_lossy(tag),
            c.tag_str()
        ))),
        Some(Err(e)) => Err(e),
        None => Err(Error::Invalid("MCNK sub-chunk missing".into())),
    }
}

fn parse_mcnk(c: &Chunk) -> Result<MapChunk> {
    let mut r = Reader::new(c.data);
    let flags = r.u32()?;
    let index_x = r.u32()?;
    let index_y = r.u32()?;
    let n_layers = r.u32()? as usize;
    let _n_doodad_refs = r.u32()?;
    let ofs_height = r.u32()?;
    let _ofs_normal = r.u32()?;
    let ofs_layer = r.u32()?;
    let _ofs_refs = r.u32()?;
    let ofs_alpha = r.u32()?;
    let size_alpha = r.u32()? as usize;
    let _ofs_shadow = r.u32()?;
    let _size_shadow = r.u32()?;
    let area_id = r.u32()?;
    let _n_map_obj_refs = r.u32()?;
    let holes = r.u32()?;
    r.skip(16)?; // low-quality texture map
    let _pred_tex = r.u32()?;
    let _no_effect_doodad = r.u32()?;
    let _ofs_snd = r.u32()?;
    let _n_snd = r.u32()?;
    let ofs_liquid = r.u32()?;
    let size_liquid = r.u32()? as usize;
    let position = r.vec3()?;

    let mut heights = [0f32; HEIGHTS_PER_CHUNK];
    let mcvt = subchunk(c, ofs_height, b"MCVT")?.ok_or_else(|| Error::Invalid("MCNK without MCVT".into()))?;
    if mcvt.len() < HEIGHTS_PER_CHUNK * 4 {
        return Err(Error::Invalid("MCVT too small".into()));
    }
    for (h, b) in heights.iter_mut().zip(mcvt.chunks_exact(4)) {
        *h = position[2] + f32::from_bits(le32(b));
    }

    let mut layers = Vec::new();
    if n_layers > 0 {
        let mcly = subchunk(c, ofs_layer, b"MCLY")?.ok_or_else(|| Error::Invalid("MCNK without MCLY".into()))?;
        let mcal = subchunk(c, ofs_alpha, b"MCAL")?.unwrap_or(&[]);
        let _ = size_alpha; // header copy of the MCAL size; the sub-chunk's own size is authoritative
        for (i, e) in mcly.chunks_exact(16).take(n_layers.min(4)).enumerate() {
            let (texture, lflags, ofs) = (le32(&e[0..]), le32(&e[4..]), le32(&e[8..]) as usize);
            let alpha = if i > 0 && lflags & 0x100 != 0 {
                // MCNK flag 0x8000 = "do not fix alpha map": otherwise the stored last
                // row/column are garbage and must be copied from their neighbours.
                mcal.get(ofs..ofs + 2048).map(|s| unpack_alpha(s, flags & 0x8000 == 0))
            } else {
                None
            };
            layers.push(Layer { texture, flags: lflags, alpha });
        }
    }

    let mut liquid = if size_liquid > 8 && ofs_liquid != 0 {
        parse_mclq(c, ofs_liquid, size_liquid)?
    } else {
        None
    };
    if let Some(l) = liquid.as_mut() {
        // MCNK flags: 0x04 river, 0x08 ocean, 0x10 magma, 0x20 slime.
        l.kind = (flags >> 2) & 0xF;
    }

    Ok(MapChunk { index_x, index_y, flags, area_id, holes, position, heights, layers, liquid })
}

/// 4-bit alpha, low nibble first, 64x64.
fn unpack_alpha(src: &[u8], fix_edges: bool) -> Box<[u8; ALPHA_DIM * ALPHA_DIM]> {
    let mut out = Box::new([0u8; ALPHA_DIM * ALPHA_DIM]);
    for (i, &b) in src.iter().enumerate().take(2048) {
        out[i * 2] = (b & 0x0F) * 17;
        out[i * 2 + 1] = (b >> 4) * 17;
    }
    if fix_edges {
        let last = ALPHA_DIM - 1;
        for y in 0..ALPHA_DIM {
            out[y * ALPHA_DIM + last] = out[y * ALPHA_DIM + last - 1];
        }
        for x in 0..ALPHA_DIM {
            out[last * ALPHA_DIM + x] = out[(last - 1) * ALPHA_DIM + x];
        }
    }
    out
}

/// MCLQ: min/max height, 9x9 vertices of 8 bytes (u32 flow/depth + f32 height),
/// then 8x8 cell flags. Only the first liquid instance is read.
///
/// The MCLQ sub-chunk's own size field is 0 in these files, so the extent comes
/// from `sizeLiquid` in the MCNK header (which includes the 8-byte chunk header).
fn parse_mclq(c: &Chunk, rel: u32, size_liquid: usize) -> Result<Option<Liquid>> {
    let start = (rel as usize).checked_sub(8).ok_or_else(|| Error::Invalid("bad MCLQ offset".into()))?;
    let Some(rest) = c.data.get(start..) else { return Ok(None) };
    if rest.len() < 8 || &[rest[3], rest[2], rest[1], rest[0]] != b"MCLQ" {
        return Ok(None);
    }
    let body = &rest[8..];
    let body = &body[..body.len().min(size_liquid.saturating_sub(8))];
    // min/max (8) + 81 vertices * 8 + 64 tile flags.
    if body.len() < 8 + 81 * 8 + 64 {
        return Ok(None);
    }
    let mut r = Reader::new(body);
    let min_height = r.f32()?;
    let max_height = r.f32()?;
    let mut heights = [0f32; 81];
    let (mut depths, mut flows) = ([0u8; 81], [0u8; 81]);
    for (i, h) in heights.iter_mut().enumerate() {
        let lead = r.bytes(4)?;
        (depths[i], flows[i]) = (lead[0], lead[1]);
        let v = r.f32()?;
        // Vertices without liquid are stored as FLT_MAX; fall back to the chunk's level.
        *h = if v.is_finite() && v.abs() < 1.0e6 { v } else { min_height };
    }
    let mut cell_flags = [0u8; 64];
    cell_flags.copy_from_slice(r.bytes(64)?);
    Ok(Some(Liquid { kind: 0, min_height, max_height, heights, depths, flows, cell_flags }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_nibbles_low_first_and_edges_fixed() {
        let mut src = vec![0u8; 2048];
        src[0] = 0xF1; // pixel 0 = 1, pixel 1 = 15
        let raw = unpack_alpha(&src, false);
        assert_eq!((raw[0], raw[1]), (17, 255));
        // With the fix, the last column copies the one before it and so does the last row.
        let mut src = vec![0u8; 2048];
        src[31] = 0x0F; // row 0: pixel 62 = 15, pixel 63 = 0
        let fixed = unpack_alpha(&src, true);
        assert_eq!(fixed[62], 255);
        assert_eq!(fixed[63], 255);
        assert_eq!(fixed[63 * ALPHA_DIM + 5], fixed[62 * ALPHA_DIM + 5]);
    }

    #[test]
    fn hole_bits_cover_two_by_two_cells() {
        let mut c = MapChunk {
            index_x: 0,
            index_y: 0,
            flags: 0,
            area_id: 0,
            holes: 1 << (1 * 4 + 2), // 4x4 grid cell (col 2, row 1)
            position: [0.0; 3],
            heights: [0.0; HEIGHTS_PER_CHUNK],
            layers: vec![],
            liquid: None,
        };
        assert!(c.is_hole(4, 2) && c.is_hole(5, 3));
        assert!(!c.is_hole(3, 2) && !c.is_hole(4, 4));
        c.holes = 0;
        assert!(!c.is_hole(4, 2));
    }
}
