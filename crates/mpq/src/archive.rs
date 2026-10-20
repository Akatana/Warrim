//! A single MPQ archive (format version 1, as shipped with WoW 1.x).

use crate::compress;
use crate::crypt::{self, HASH_NAME_A, HASH_NAME_B, HASH_TABLE_INDEX};
use crate::{Error, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAGIC: u32 = 0x1A51_504D; // "MPQ\x1A"

pub const FLAG_IMPLODE: u32 = 0x0000_0100;
pub const FLAG_COMPRESS: u32 = 0x0000_0200;
pub const FLAG_ENCRYPTED: u32 = 0x0001_0000;
pub const FLAG_FIX_KEY: u32 = 0x0002_0000;
pub const FLAG_SINGLE_UNIT: u32 = 0x0100_0000;
pub const FLAG_DELETE_MARKER: u32 = 0x0200_0000;
pub const FLAG_SECTOR_CRC: u32 = 0x0400_0000;
pub const FLAG_EXISTS: u32 = 0x8000_0000;

const HASH_EMPTY: u32 = 0xFFFF_FFFF;
const HASH_DELETED: u32 = 0xFFFF_FFFE;

#[derive(Clone, Copy, Debug)]
struct HashEntry {
    name_a: u32,
    name_b: u32,
    locale: u16,
    block: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct BlockEntry {
    pub pos: u32,
    pub packed_size: u32,
    pub file_size: u32,
    pub flags: u32,
}

pub struct Archive {
    path: PathBuf,
    file: Mutex<File>,
    base: u64,
    sector_size: usize,
    hashes: Vec<HashEntry>,
    blocks: Vec<BlockEntry>,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

impl Archive {
    pub fn open(path: impl AsRef<Path>) -> Result<Archive> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let len = file.metadata()?.len();

        // The header sits on a 512-byte boundary; in practice it is at offset 0.
        let mut base = 0u64;
        let mut hdr = [0u8; 32];
        loop {
            if base + 32 > len {
                return Err(Error::Corrupt("no MPQ header found"));
            }
            file.seek(SeekFrom::Start(base))?;
            file.read_exact(&mut hdr)?;
            if u32le(&hdr, 0) == MAGIC {
                break;
            }
            base += 512;
        }

        let block_shift = u16le(&hdr, 14) as u32;
        let hash_pos = u32le(&hdr, 16) as u64;
        let block_pos = u32le(&hdr, 20) as u64;
        let hash_len = u32le(&hdr, 24) as usize;
        let block_len = u32le(&hdr, 28) as usize;
        if !hash_len.is_power_of_two() {
            return Err(Error::Corrupt("hash table size is not a power of two"));
        }

        let mut read_table = |pos: u64, entries: usize, key_name: &str| -> Result<Vec<u8>> {
            let mut buf = vec![0u8; entries * 16];
            file.seek(SeekFrom::Start(base + pos))?;
            file.read_exact(&mut buf)?;
            crypt::decrypt_bytes(&mut buf, crypt::hash_string(key_name, crypt::HASH_FILE_KEY));
            Ok(buf)
        };
        let hash_raw = read_table(hash_pos, hash_len, "(hash table)")?;
        let block_raw = read_table(block_pos, block_len, "(block table)")?;

        let hashes = hash_raw
            .chunks_exact(16)
            .map(|c| HashEntry {
                name_a: u32le(c, 0),
                name_b: u32le(c, 4),
                locale: u16le(c, 8),
                block: u32le(c, 12),
            })
            .collect();
        let blocks = block_raw
            .chunks_exact(16)
            .map(|c| BlockEntry {
                pos: u32le(c, 0),
                packed_size: u32le(c, 4),
                file_size: u32le(c, 8),
                flags: u32le(c, 12),
            })
            .collect();

        Ok(Archive {
            path,
            file: Mutex::new(file),
            base,
            sector_size: 512usize << block_shift,
            hashes,
            blocks,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_count(&self) -> usize {
        self.blocks.len()
    }

    /// Look up the block describing `name`, preferring the neutral locale.
    pub fn find(&self, name: &str) -> Option<BlockEntry> {
        let mask = self.hashes.len() - 1;
        let start = crypt::hash_string(name, HASH_TABLE_INDEX) as usize & mask;
        let a = crypt::hash_string(name, HASH_NAME_A);
        let b = crypt::hash_string(name, HASH_NAME_B);

        let mut best: Option<(u16, u32)> = None;
        let mut i = start;
        loop {
            let e = self.hashes[i];
            if e.block == HASH_EMPTY {
                break;
            }
            if e.block != HASH_DELETED && e.name_a == a && e.name_b == b {
                if e.locale == 0 {
                    best = Some((e.locale, e.block));
                    break;
                }
                best.get_or_insert((e.locale, e.block));
            }
            i = (i + 1) & mask;
            if i == start {
                break;
            }
        }
        let (_, block) = best?;
        self.blocks.get(block as usize).copied()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// Read and fully decode a file. `None` if absent from this archive.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(blk) = self.find(name) else {
            return Ok(None);
        };
        if blk.flags & FLAG_EXISTS == 0 || blk.flags & FLAG_DELETE_MARKER != 0 {
            return Ok(None);
        }
        self.read_block(name, blk).map(Some)
    }

    /// True if this archive carries a tombstone for `name`.
    pub fn is_deleted(&self, name: &str) -> bool {
        self.find(name)
            .map(|b| b.flags & FLAG_DELETE_MARKER != 0 || b.flags & FLAG_EXISTS == 0)
            .unwrap_or(false)
    }

    fn read_at(&self, pos: u64, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        let mut f = self.file.lock().unwrap();
        f.seek(SeekFrom::Start(self.base + pos))?;
        f.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn read_block(&self, name: &str, blk: BlockEntry) -> Result<Vec<u8>> {
        let size = blk.file_size as usize;
        if size == 0 {
            return Ok(Vec::new());
        }
        let encrypted = blk.flags & FLAG_ENCRYPTED != 0;
        let compressed = blk.flags & (FLAG_COMPRESS | FLAG_IMPLODE) != 0;
        let key = if encrypted {
            crypt::file_key(name, blk.pos, blk.file_size, blk.flags & FLAG_FIX_KEY != 0)
        } else {
            0
        };

        if blk.flags & FLAG_SINGLE_UNIT != 0 {
            let mut data = self.read_at(blk.pos as u64, blk.packed_size as usize)?;
            if encrypted {
                crypt::decrypt_bytes(&mut data, key);
            }
            return if compressed && data.len() < size {
                self.decode_sector(&data, size, blk.flags)
            } else {
                Ok(data)
            };
        }

        let n_sectors = size.div_ceil(self.sector_size);

        // Offsets of each sector relative to the start of the file data.
        let offsets: Vec<usize> = if compressed {
            let crc_extra = usize::from(blk.flags & FLAG_SECTOR_CRC != 0);
            let mut raw = self.read_at(blk.pos as u64, (n_sectors + 1 + crc_extra) * 4)?;
            if encrypted {
                crypt::decrypt_bytes(&mut raw, key.wrapping_sub(1));
            }
            (0..=n_sectors).map(|i| u32le(&raw, i * 4) as usize).collect()
        } else {
            (0..=n_sectors)
                .map(|i| (i * self.sector_size).min(size))
                .collect()
        };

        // One contiguous read for the whole file body.
        let first = offsets[0];
        let last = offsets[n_sectors];
        if last < first || last > blk.packed_size.max(blk.file_size) as usize + 4 {
            return Err(Error::Corrupt("sector offset table"));
        }
        let body = self.read_at(blk.pos as u64 + first as u64, last - first)?;

        let mut out = Vec::with_capacity(size);
        for i in 0..n_sectors {
            let (s, e) = (offsets[i] - first, offsets[i + 1] - first);
            if e < s || e > body.len() {
                return Err(Error::Corrupt("sector bounds"));
            }
            let mut sector = body[s..e].to_vec();
            if encrypted {
                crypt::decrypt_bytes(&mut sector, key.wrapping_add(i as u32));
            }
            let want = (size - i * self.sector_size).min(self.sector_size);
            if compressed && sector.len() < want {
                out.extend(self.decode_sector(&sector, want, blk.flags)?);
            } else {
                out.extend(sector);
            }
        }
        if out.len() != size {
            return Err(Error::Corrupt("decoded size mismatch"));
        }
        Ok(out)
    }

    fn decode_sector(&self, data: &[u8], want: usize, flags: u32) -> Result<Vec<u8>> {
        let out = if flags & FLAG_COMPRESS != 0 {
            compress::decompress_masked(data, want)?
        } else {
            compress::pkware(data, want)?
        };
        if out.len() != want {
            return Err(Error::Corrupt("sector decompressed to wrong length"));
        }
        Ok(out)
    }

    /// Contents of the `(listfile)` entry, if the archive has one.
    pub fn listfile(&self) -> Vec<String> {
        match self.read("(listfile)") {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }
}
