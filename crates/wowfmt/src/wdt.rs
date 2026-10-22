//! WDT: which 533.33-yard ADT tiles exist for a map.

use crate::reader::{Chunks, Reader};
use crate::{Error, Result};

pub const TILES: usize = 64;

/// The tile a global WMO map keeps its one building in: the placement is measured from the origin of the map, which is the middle of the grid.
pub const GLOBAL_TILE: usize = 32;

/// The single building of a map with no terrain (a dungeon): MWMO's name and MODF's placement, in render-space yards (not shifted by the map's corner).
pub struct GlobalWmo {
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub doodad_set: u16,
}

pub struct Wdt {
    /// `has_adt[y][x]`; `x` is the first number in `Map_x_y.adt`.
    has_adt: Vec<bool>,
    /// MPHD flags. Bit 0 = the map is a single WMO ("global WMO") with no terrain.
    pub flags: u32,
    pub global: Option<GlobalWmo>,
}

impl Wdt {
    pub fn parse(data: &[u8]) -> Result<Wdt> {
        let mut has_adt = vec![false; TILES * TILES];
        let mut flags = 0;
        let mut found_main = false;
        let mut name = String::new();
        let mut placement = None;
        for chunk in Chunks::new(data) {
            let c = chunk?;
            match &c.tag {
                b"MPHD" if c.data.len() >= 4 => {
                    flags = u32::from_le_bytes([c.data[0], c.data[1], c.data[2], c.data[3]]);
                }
                b"MAIN" => {
                    if c.data.len() < TILES * TILES * 8 {
                        return Err(Error::Invalid("MAIN chunk too small".into()));
                    }
                    for (i, e) in c.data.chunks_exact(8).take(TILES * TILES).enumerate() {
                        // Entries are stored row-major: index = y * 64 + x.
                        has_adt[i] = u32::from_le_bytes([e[0], e[1], e[2], e[3]]) & 1 != 0;
                    }
                    found_main = true;
                }
                b"MWMO" => name = String::from_utf8_lossy(c.data.split(|&b| b == 0).next().unwrap_or_default()).into_owned(),
                b"MODF" if c.data.len() >= 64 => {
                    let mut r = Reader::new(c.data);
                    r.skip(8)?; // name index, unique id
                    let (position, rotation) = (r.vec3()?, r.vec3()?);
                    r.skip(24)?; // bounding box
                    r.skip(2)?; // flags
                    placement = Some((position, rotation, r.u16()?));
                }
                _ => {}
            }
        }
        if !found_main {
            return Err(Error::Invalid("WDT has no MAIN chunk".into()));
        }
        let global = placement.filter(|_| flags & 1 != 0 && !name.is_empty()).map(|(position, rotation, doodad_set)| GlobalWmo { name, position, rotation, doodad_set });
        if global.is_some() {
            has_adt[GLOBAL_TILE * TILES + GLOBAL_TILE] = true;
        }
        Ok(Wdt { has_adt, flags, global })
    }

    pub fn has_tile(&self, x: usize, y: usize) -> bool {
        x < TILES && y < TILES && self.has_adt[y * TILES + x]
    }

    pub fn tile_count(&self) -> usize {
        self.has_adt.iter().filter(|&&b| b).count()
    }
}
