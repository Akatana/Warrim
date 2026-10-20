//! BLP2 textures (the only flavour used by the 1.12 client).
//!
//! Encodings: 1 = 256-colour palette (+ 0/1/4/8-bit alpha plane),
//!            2 = DXT1 / DXT3 / DXT5,  3 = raw BGRA.

use crate::reader::Reader;
use crate::{Error, Result};

pub struct Blp {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    /// RGBA8 pixels, one entry per stored mip level (level 0 first).
    pub mips: Vec<Vec<u8>>,
}

impl Blp {
    pub fn level_size(&self, level: usize) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }

    pub fn parse(data: &[u8]) -> Result<Blp> {
        let mut r = Reader::new(data);
        let magic = r.bytes(4)?;
        if magic != b"BLP2" {
            return Err(Error::Magic { expected: "BLP2", found: String::from_utf8_lossy(magic).into() });
        }
        let _kind = r.u32()?;
        let encoding = r.u8()?;
        let alpha_depth = r.u8()?;
        let alpha_encoding = r.u8()?;
        let has_mips = r.u8()? != 0;
        let width = r.u32()?;
        let height = r.u32()?;
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(Error::Invalid(format!("BLP size {width}x{height}")));
        }
        let mut offsets = [0u32; 16];
        let mut sizes = [0u32; 16];
        for o in offsets.iter_mut() {
            *o = r.u32()?;
        }
        for s in sizes.iter_mut() {
            *s = r.u32()?;
        }
        let mut palette = [[0u8; 4]; 256];
        for p in palette.iter_mut() {
            let b = r.bytes(4)?;
            *p = [b[2], b[1], b[0], b[3]]; // BGRA -> RGBA
        }

        let max_levels = 32 - width.max(height).leading_zeros() as usize;
        let levels = if has_mips { max_levels.min(16) } else { 1 };
        let mut mips = Vec::new();
        for level in 0..levels {
            if offsets[level] == 0 || sizes[level] == 0 {
                break;
            }
            let (w, h) = (((width >> level).max(1)) as usize, ((height >> level).max(1)) as usize);
            let start = offsets[level] as usize;
            let end = start + sizes[level] as usize;
            let src = data
                .get(start..end)
                .ok_or(Error::Eof { at: start, wanted: sizes[level] as usize, len: data.len() })?;
            let decoded = match encoding {
                1 => decode_palette(src, w, h, &palette, alpha_depth),
                2 => decode_dxt(src, w, h, alpha_encoding),
                3 => decode_bgra(src, w, h),
                e => return Err(Error::Unsupported(format!("BLP encoding {e}"))),
            };
            match decoded {
                Ok(rgba) => mips.push(rgba),
                // Some files store their smallest mips short; the chain simply ends there.
                Err(_) if level > 0 => break,
                Err(e) => return Err(e),
            }
        }
        if mips.is_empty() {
            return Err(Error::Invalid("BLP has no mip data".into()));
        }
        Ok(Blp { width, height, has_alpha: alpha_depth > 0, mips })
    }
}

fn decode_palette(src: &[u8], w: usize, h: usize, pal: &[[u8; 4]; 256], alpha_depth: u8) -> Result<Vec<u8>> {
    let n = w * h;
    let alpha_bytes = match alpha_depth {
        0 => 0,
        1 => n.div_ceil(8),
        4 => n.div_ceil(2),
        8 => n,
        d => return Err(Error::Unsupported(format!("BLP alpha depth {d}"))),
    };
    if src.len() < n + alpha_bytes {
        return Err(Error::Eof { at: 0, wanted: n + alpha_bytes, len: src.len() });
    }
    let (idx, alpha) = src.split_at(n);
    let mut out = Vec::with_capacity(n * 4);
    for (i, &p) in idx.iter().enumerate() {
        let c = pal[p as usize];
        let a = match alpha_depth {
            0 => 255,
            1 => ((alpha[i / 8] >> (i % 8)) & 1) * 255,
            4 => {
                let v = (alpha[i / 2] >> ((i % 2) * 4)) & 0xF;
                v * 17
            }
            _ => alpha[i],
        };
        out.extend_from_slice(&[c[0], c[1], c[2], a]);
    }
    Ok(out)
}

fn decode_bgra(src: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    if src.len() < w * h * 4 {
        return Err(Error::Eof { at: 0, wanted: w * h * 4, len: src.len() });
    }
    Ok(src[..w * h * 4].chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect())
}

fn rgb565(v: u16) -> [u8; 3] {
    let r = ((v >> 11) & 31) as u32;
    let g = ((v >> 5) & 63) as u32;
    let b = (v & 31) as u32;
    [((r * 255 + 15) / 31) as u8, ((g * 255 + 31) / 63) as u8, ((b * 255 + 15) / 31) as u8]
}

/// Decode a DXT1 colour block into 16 RGBA pixels. `punch_through` enables the
/// 1-bit-alpha mode of DXT1 (only valid when the format is really DXT1).
fn dxt_color_block(b: &[u8], punch_through: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || !punch_through {
        for i in 0..3 {
            pal[2][i] = ((2 * p0[i] as u32 + p1[i] as u32) / 3) as u8;
            pal[3][i] = ((p0[i] as u32 + 2 * p1[i] as u32) / 3) as u8;
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for i in 0..3 {
            pal[2][i] = ((p0[i] as u32 + p1[i] as u32) / 2) as u8;
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let mut out = [[0u8; 4]; 16];
    for (i, px) in out.iter_mut().enumerate() {
        *px = pal[((bits >> (2 * i)) & 3) as usize];
    }
    out
}

fn decode_dxt(src: &[u8], w: usize, h: usize, alpha_encoding: u8) -> Result<Vec<u8>> {
    let (block_size, kind) = match alpha_encoding {
        0 => (8, 1),
        1 => (16, 3),
        7 => (16, 5),
        e => return Err(Error::Unsupported(format!("BLP alpha encoding {e}"))),
    };
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    if src.len() < bw * bh * block_size {
        return Err(Error::Eof { at: 0, wanted: bw * bh * block_size, len: src.len() });
    }
    let mut out = vec![0u8; w * h * 4];
    for by in 0..bh {
        for bx in 0..bw {
            let blk = &src[(by * bw + bx) * block_size..][..block_size];
            let mut px = match kind {
                1 => dxt_color_block(blk, true),
                _ => dxt_color_block(&blk[8..], false),
            };
            match kind {
                3 => {
                    for i in 0..16 {
                        let v = (blk[i / 2] >> ((i % 2) * 4)) & 0xF;
                        px[i][3] = v * 17;
                    }
                }
                5 => {
                    let (a0, a1) = (blk[0] as u32, blk[1] as u32);
                    let mut lut = [0u8; 8];
                    lut[0] = a0 as u8;
                    lut[1] = a1 as u8;
                    if a0 > a1 {
                        for i in 1..7u32 {
                            lut[(i + 1) as usize] = (((7 - i) * a0 + i * a1) / 7) as u8;
                        }
                    } else {
                        for i in 1..5u32 {
                            lut[(i + 1) as usize] = (((5 - i) * a0 + i * a1) / 5) as u8;
                        }
                        lut[6] = 0;
                        lut[7] = 255;
                    }
                    let mut bits = 0u64;
                    for i in 0..6 {
                        bits |= (blk[2 + i] as u64) << (8 * i);
                    }
                    for (i, p) in px.iter_mut().enumerate() {
                        p[3] = lut[((bits >> (3 * i)) & 7) as usize];
                    }
                }
                _ => {}
            }
            for (i, p) in px.iter().enumerate() {
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                if x < w && y < h {
                    out[(y * w + x) * 4..][..4].copy_from_slice(p);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BLP2 header + palette with `levels` mips of DXT data, each `sizes[i]` bytes of `fill`.
    fn dxt_blp(alpha_encoding: u8, width: u32, height: u32, sizes: &[usize], fill: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"BLP2");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&[2, 8, alpha_encoding, 1]);
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        let data_start = 148 + 1024;
        let mut offsets = [0u32; 16];
        let mut lens = [0u32; 16];
        let mut pos = data_start;
        for (i, &s) in sizes.iter().enumerate() {
            offsets[i] = pos as u32;
            lens[i] = s as u32;
            pos += s;
        }
        for o in offsets {
            out.extend_from_slice(&o.to_le_bytes());
        }
        for l in lens {
            out.extend_from_slice(&l.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 1024]);
        for &s in sizes {
            out.extend((0..s).map(|i| fill[i % fill.len()]));
        }
        out
    }

    #[test]
    fn dxt1_solid_colour_block() {
        // c0 = c1 = pure red (0xF800), all indices 0.
        let block = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0];
        let blp = Blp::parse(&dxt_blp(0, 4, 4, &[8], &block)).unwrap();
        assert_eq!(blp.mips.len(), 1);
        for px in blp.mips[0].chunks_exact(4) {
            assert_eq!(px, [255, 0, 0, 255]);
        }
    }

    #[test]
    fn dxt5_alpha_endpoints() {
        // alpha0 = 255, alpha1 = 0, every index 1 -> alpha 0; colour block solid green.
        let mut block = vec![255, 0, 0x49, 0x92, 0x24, 0x49, 0x92, 0x24];
        block.extend_from_slice(&[0xE0, 0x07, 0xE0, 0x07, 0, 0, 0, 0]);
        // indices 0b001 repeated -> bytes 0x49 0x92 0x24 ...
        let blp = Blp::parse(&dxt_blp(7, 4, 4, &[16], &block)).unwrap();
        for px in blp.mips[0].chunks_exact(4) {
            assert_eq!(&px[..3], [0, 255, 0]);
            assert_eq!(px[3], 0);
        }
    }

    /// Regression: some DXT3/5 files store their smallest mips at half size. That used to
    /// reject the whole texture; now the mip chain just ends there.
    #[test]
    fn short_tail_mips_keep_the_texture() {
        let block = vec![0u8; 16];
        // 8x8 DXT3: level 0 = 4 blocks (64 bytes), level 1 (4x4) = 16, level 2 (2x2) truncated to 8.
        let blp = Blp::parse(&dxt_blp(1, 8, 8, &[64, 16, 8], &block)).unwrap();
        assert_eq!(blp.mips.len(), 2);
        assert_eq!(blp.mips[0].len(), 8 * 8 * 4);
        assert_eq!(blp.mips[1].len(), 4 * 4 * 4);
    }

    #[test]
    fn truncated_base_level_is_an_error() {
        let block = vec![0u8; 16];
        assert!(Blp::parse(&dxt_blp(1, 8, 8, &[32], &block)).is_err());
    }
}
