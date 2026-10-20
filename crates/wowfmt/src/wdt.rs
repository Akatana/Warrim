//! WDT: which 533.33-yard ADT tiles exist for a map.

use crate::reader::Chunks;
use crate::{Error, Result};

pub const TILES: usize = 64;

pub struct Wdt {
    /// `has_adt[y][x]`; `x` is the first number in `Map_x_y.adt`.
    has_adt: Vec<bool>,
    /// MPHD flags. Bit 0 = the map is a single WMO ("global WMO") with no terrain.
    pub flags: u32,
}

impl Wdt {
    pub fn parse(data: &[u8]) -> Result<Wdt> {
        let mut has_adt = vec![false; TILES * TILES];
        let mut flags = 0;
        let mut found_main = false;
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
                _ => {}
            }
        }
        if !found_main {
            return Err(Error::Invalid("WDT has no MAIN chunk".into()));
        }
        Ok(Wdt { has_adt, flags })
    }

    pub fn has_tile(&self, x: usize, y: usize) -> bool {
        x < TILES && y < TILES && self.has_adt[y * TILES + x]
    }

    pub fn tile_count(&self) -> usize {
        self.has_adt.iter().filter(|&&b| b).count()
    }
}
