//! Sector decompression.

use crate::{Error, Result};
use std::io::Read;

const COMP_HUFFMAN: u8 = 0x01;
const COMP_ZLIB: u8 = 0x02;
const COMP_PKWARE: u8 = 0x08;
const COMP_BZIP2: u8 = 0x10;
const COMP_SPARSE: u8 = 0x20;
const COMP_ADPCM_MONO: u8 = 0x40;
const COMP_ADPCM_STEREO: u8 = 0x80;

/// Decompress a sector of a file stored with the generic COMPRESS flag: the first
/// byte is a bitmask of the algorithms that were applied.
pub fn decompress_masked(src: &[u8], out_len: usize) -> Result<Vec<u8>> {
    let (&mask, payload) = src.split_first().ok_or(Error::Corrupt("empty compressed sector"))?;
    let unsupported = mask & (COMP_HUFFMAN | COMP_SPARSE | COMP_ADPCM_MONO | COMP_ADPCM_STEREO);
    if unsupported != 0 {
        return Err(Error::Unsupported(match unsupported {
            COMP_HUFFMAN => "huffman compression",
            COMP_SPARSE => "sparse compression",
            _ => "ADPCM audio compression",
        }));
    }
    match mask {
        COMP_ZLIB => zlib(payload, out_len),
        COMP_PKWARE => pkware(payload, out_len),
        COMP_BZIP2 => bzip2(payload, out_len),
        // Multiple algorithms chained: bzip2 -> pkware -> zlib (order used by StormLib).
        m if m & (COMP_ZLIB | COMP_PKWARE | COMP_BZIP2) != 0 => {
            let mut data = payload.to_vec();
            if m & COMP_BZIP2 != 0 {
                data = bzip2(&data, out_len)?;
            }
            if m & COMP_PKWARE != 0 {
                data = pkware(&data, out_len)?;
            }
            if m & COMP_ZLIB != 0 {
                data = zlib(&data, out_len)?;
            }
            Ok(data)
        }
        _ => Err(Error::Unsupported("unknown compression mask")),
    }
}

pub fn zlib(src: &[u8], out_len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(out_len);
    flate2::read::ZlibDecoder::new(src)
        .read_to_end(&mut out)
        .map_err(|_| Error::Corrupt("zlib stream"))?;
    Ok(out)
}

pub fn bzip2(src: &[u8], out_len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(out_len);
    bzip2::read::BzDecoder::new(src)
        .read_to_end(&mut out)
        .map_err(|_| Error::Corrupt("bzip2 stream"))?;
    Ok(out)
}

/// PKWARE Data Compression Library "implode" stream.
pub fn pkware(src: &[u8], _out_len: usize) -> Result<Vec<u8>> {
    explode::explode(src).map_err(|_| Error::Corrupt("pkware implode stream"))
}
