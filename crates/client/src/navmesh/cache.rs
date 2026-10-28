//! The tiles of the navigation mesh that were built, kept in a file of the application folder (one per map) so that the next run reads them instead of building them
//! again. The file is a list of records (the tile, then its bytes; a record without bytes says there is nothing to walk on there), added as tiles are built. It carries
//! a stamp of what the tiles were built from (the builder and the props of the content): a file with another stamp is thrown away.

use super::mesh::{MeshTile, Poly, MAX_VERTS};
use glam::Vec3;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"RSNV";
/// Changes with the numbers of the builder and the form of a tile in the file.
pub const VERSION: u32 = 1;
const HEADER: u64 = 16;
const RECORD: u64 = 12;

pub struct NavCache {
    file: File,
    /// Per tile (north, west): where its bytes are in the file, and how many (none: nothing to walk on).
    index: HashMap<(i32, i32), (u64, u32)>,
}

impl NavCache {
    /// Open the file (made if there is none); one with another stamp, or a damaged tail, is cut back.
    pub fn open(path: &Path, stamp: u64) -> std::io::Result<NavCache> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        let mut header = [0u8; HEADER as usize];
        let fresh = file.read_exact(&mut header).is_err() || &header[..4] != MAGIC || u32::from_le_bytes(header[4..8].try_into().expect("four bytes")) != VERSION || u64::from_le_bytes(header[8..16].try_into().expect("eight bytes")) != stamp;
        let mut index = HashMap::new();
        if fresh {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(MAGIC)?;
            file.write_all(&VERSION.to_le_bytes())?;
            file.write_all(&stamp.to_le_bytes())?;
        } else {
            let length = file.metadata()?.len();
            let mut at = HEADER;
            while at + RECORD <= length {
                file.seek(SeekFrom::Start(at))?;
                let mut record = [0u8; RECORD as usize];
                file.read_exact(&mut record)?;
                let key = (i32::from_le_bytes(record[0..4].try_into().expect("four bytes")), i32::from_le_bytes(record[4..8].try_into().expect("four bytes")));
                let size = u32::from_le_bytes(record[8..12].try_into().expect("four bytes"));
                if at + RECORD + size as u64 > length {
                    break;
                }
                index.insert(key, (at + RECORD, size));
                at += RECORD + size as u64;
            }
            // (A record cut short by a crash goes.)
            if at < length {
                file.set_len(at)?;
            }
        }
        Ok(NavCache { file, index })
    }

    /// The tile `(north, west)` as it was kept: `Some(None)` when there was nothing to walk on, `None` when it was not kept (or cannot be read).
    pub fn load(&mut self, north: i32, west: i32) -> Option<Option<MeshTile>> {
        let &(at, size) = self.index.get(&(north, west))?;
        if size == 0 {
            return Some(None);
        }
        let mut bytes = vec![0u8; size as usize];
        self.file.seek(SeekFrom::Start(at)).ok()?;
        self.file.read_exact(&mut bytes).ok()?;
        decode(&bytes).map(Some)
    }

    /// Keep the tile `(north, west)` (`None`: there is nothing to walk on there).
    pub fn save(&mut self, north: i32, west: i32, tile: Option<&MeshTile>) -> std::io::Result<()> {
        let bytes = tile.map(encode).unwrap_or_default();
        let end = self.file.seek(SeekFrom::End(0))?;
        let mut record = Vec::with_capacity(RECORD as usize + bytes.len());
        record.extend_from_slice(&north.to_le_bytes());
        record.extend_from_slice(&west.to_le_bytes());
        record.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        record.extend_from_slice(&bytes);
        self.file.write_all(&record)?;
        self.index.insert((north, west), (end + RECORD, bytes.len() as u32));
        Ok(())
    }
}

fn encode(tile: &MeshTile) -> Vec<u8> {
    let mut out = Vec::new();
    let int = |out: &mut Vec<u8>, v: i32| out.extend_from_slice(&v.to_le_bytes());
    let float3 = |out: &mut Vec<u8>, v: Vec3| v.to_array().iter().for_each(|f| out.extend_from_slice(&f.to_le_bytes()));
    int(&mut out, tile.x);
    int(&mut out, tile.y);
    out.extend_from_slice(&(tile.verts.len() as u32).to_le_bytes());
    tile.verts.iter().for_each(|&v| float3(&mut out, v));
    out.extend_from_slice(&(tile.polys.len() as u32).to_le_bytes());
    for (poly, (verts, tris)) in tile.polys.iter().zip(&tile.detail) {
        out.push(poly.count as u8);
        poly.verts.iter().chain(&poly.neis).for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        out.push(poly.area);
        out.extend_from_slice(&poly.flags.to_le_bytes());
        out.extend_from_slice(&(verts.len() as u16).to_le_bytes());
        verts.iter().for_each(|&v| float3(&mut out, v));
        out.extend_from_slice(&(tris.len() as u16).to_le_bytes());
        tris.iter().for_each(|t| out.extend_from_slice(t));
    }
    out
}

/// Reads the bytes in order; `None` once they run out.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits)
    }
    fn vec3(&mut self) -> Option<Vec3> {
        Some(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
}

fn decode(bytes: &[u8]) -> Option<MeshTile> {
    let mut r = Reader(bytes);
    let (x, y) = (r.u32()? as i32, r.u32()? as i32);
    let verts = (0..r.u32()?).map(|_| r.vec3()).collect::<Option<Vec<_>>>()?;
    let mut polys = Vec::new();
    let mut detail = Vec::new();
    for _ in 0..r.u32()? {
        let count = r.u8()? as usize;
        let mut poly = Poly { verts: [0; MAX_VERTS], neis: [0; MAX_VERTS], count, area: 0, flags: 0 };
        for v in poly.verts.iter_mut() {
            *v = r.u16()?;
        }
        for n in poly.neis.iter_mut() {
            *n = r.u16()?;
        }
        (poly.area, poly.flags) = (r.u8()?, r.u16()?);
        if count > MAX_VERTS || poly.verts[..count].iter().any(|&v| v as usize >= verts.len()) {
            return None;
        }
        let corners = (0..r.u16()?).map(|_| r.vec3()).collect::<Option<Vec<_>>>()?;
        let tris = (0..r.u16()?).map(|_| Some([r.u8()?, r.u8()?, r.u8()?])).collect::<Option<Vec<_>>>()?;
        if tris.iter().flatten().any(|&i| i as usize >= corners.len()) {
            return None;
        }
        polys.push(poly);
        detail.push((corners, tris));
    }
    // (A polygon that leads to one the tile does not have is a damaged file.)
    if polys.iter().any(|p| p.neis[..p.count].iter().any(|&n| n != 0 && n & super::mesh::EXT_LINK == 0 && n as usize > polys.len())) {
        return None;
    }
    let links = vec![Vec::new(); polys.len()];
    Some(MeshTile { x, y, verts, polys, links, detail })
}
