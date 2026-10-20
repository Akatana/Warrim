//! MPQ hashing and the proprietary block cipher.

use std::sync::OnceLock;

pub const HASH_TABLE_INDEX: u32 = 0;
pub const HASH_NAME_A: u32 = 1;
pub const HASH_NAME_B: u32 = 2;
pub const HASH_FILE_KEY: u32 = 3;

fn table() -> &'static [u32; 0x500] {
    static TABLE: OnceLock<[u32; 0x500]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 0x500];
        let mut seed: u32 = 0x0010_0001;
        for i in 0..0x100 {
            let mut idx = i;
            for _ in 0..5 {
                seed = (seed * 125 + 3) % 0x2A_AAAB;
                let hi = (seed & 0xFFFF) << 16;
                seed = (seed * 125 + 3) % 0x2A_AAAB;
                let lo = seed & 0xFFFF;
                t[idx] = hi | lo;
                idx += 0x100;
            }
        }
        t
    })
}

/// Hash a path. Case-insensitive; `/` and `\` are equivalent.
pub fn hash_string(name: &str, hash_type: u32) -> u32 {
    let t = table();
    let mut seed1: u32 = 0x7FED_7FED;
    let mut seed2: u32 = 0xEEEE_EEEE;
    for b in name.bytes() {
        let ch = match b {
            b'/' => b'\\',
            b'a'..=b'z' => b - 32,
            _ => b,
        } as u32;
        seed1 = t[((hash_type << 8) + ch) as usize] ^ seed1.wrapping_add(seed2);
        seed2 = ch
            .wrapping_add(seed1)
            .wrapping_add(seed2)
            .wrapping_add(seed2 << 5)
            .wrapping_add(3);
    }
    seed1
}

pub fn decrypt_block(data: &mut [u32], mut key: u32) {
    let t = table();
    let mut seed: u32 = 0xEEEE_EEEE;
    for d in data.iter_mut() {
        seed = seed.wrapping_add(t[0x400 + (key & 0xFF) as usize]);
        let ch = *d ^ key.wrapping_add(seed);
        *d = ch;
        key = ((!key) << 0x15).wrapping_add(0x1111_1111) | (key >> 0x0B);
        seed = ch
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
    }
}

/// Decrypt a little-endian byte buffer in place. Trailing bytes that do not
/// fill a whole u32 are left untouched (they are not encrypted).
pub fn decrypt_bytes(data: &mut [u8], key: u32) {
    let words = data.len() / 4;
    let mut tmp: Vec<u32> = data[..words * 4]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    decrypt_block(&mut tmp, key);
    for (c, w) in data[..words * 4].chunks_exact_mut(4).zip(tmp) {
        c.copy_from_slice(&w.to_le_bytes());
    }
}

/// File name -> encryption key (only the final path component participates).
pub fn file_key(name: &str, block_pos: u32, file_size: u32, fix_key: bool) -> u32 {
    let base = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let mut key = hash_string(base, HASH_FILE_KEY);
    if fix_key {
        key = key.wrapping_add(block_pos) ^ file_size;
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_table_hashes() {
        // Values from StormLib / the MPQ format documentation.
        assert_eq!(hash_string("(hash table)", HASH_FILE_KEY), 0xC3AF_3770);
        assert_eq!(hash_string("(block table)", HASH_FILE_KEY), 0xEC83_B3A3);
    }

    #[test]
    fn case_and_slash_insensitive() {
        assert_eq!(
            hash_string("World/Maps/Azeroth/Azeroth.wdt", HASH_NAME_A),
            hash_string("WORLD\\MAPS\\AZEROTH\\AZEROTH.WDT", HASH_NAME_A)
        );
    }
}
