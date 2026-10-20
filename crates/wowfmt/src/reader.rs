//! Bounds-checked little-endian reader.

use crate::{Error, Result};

#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n);
        match end {
            Some(end) if end <= self.data.len() => {
                let s = &self.data[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            _ => Err(Error::Eof { at: self.pos, wanted: n, len: self.data.len() }),
        }
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn vec3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
}

/// Iterates the `FOURCC + u32 size + payload` chunks used by ADT/WDT/WMO/M2 files.
/// WoW stores the tags byte-reversed on disk ("REVM" for MVER); they are
/// normalised here so callers compare against the readable form.
pub struct Chunks<'a> {
    r: Reader<'a>,
}

#[derive(Clone, Copy)]
pub struct Chunk<'a> {
    pub tag: [u8; 4],
    pub data: &'a [u8],
    /// Offset of the chunk *header* inside the buffer that was iterated.
    pub offset: usize,
}

impl Chunk<'_> {
    pub fn tag_str(&self) -> String {
        String::from_utf8_lossy(&self.tag).into_owned()
    }
}

impl<'a> Chunks<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { r: Reader::new(data) }
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.r.remaining() < 8 {
            return None;
        }
        let offset = self.r.pos();
        let res = (|| {
            let t = self.r.bytes(4)?;
            let size = self.r.u32()? as usize;
            let data = self.r.bytes(size)?;
            Ok(Chunk { tag: [t[3], t[2], t[1], t[0]], data, offset })
        })();
        Some(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_tags_are_normalised_and_sizes_respected() {
        // "REVM" on disk is MVER; payload 4 bytes. Then "KNCM" (MCNK) with 2 bytes.
        let mut d = Vec::new();
        d.extend_from_slice(b"REVM");
        d.extend_from_slice(&4u32.to_le_bytes());
        d.extend_from_slice(&18u32.to_le_bytes());
        d.extend_from_slice(b"KNCM");
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&[7, 9]);
        let chunks: Vec<_> = Chunks::new(&d).collect::<Result<_>>().unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(&chunks[0].tag, b"MVER");
        assert_eq!(chunks[0].data, 18u32.to_le_bytes());
        assert_eq!(&chunks[1].tag, b"MCNK");
        assert_eq!(chunks[1].data, [7, 9]);
        assert_eq!(chunks[1].offset, 12);
    }

    #[test]
    fn chunk_running_past_the_end_is_an_error_not_a_panic() {
        let mut d = Vec::new();
        d.extend_from_slice(b"REVM");
        d.extend_from_slice(&100u32.to_le_bytes());
        assert!(Chunks::new(&d).next().unwrap().is_err());
    }

    #[test]
    fn reader_is_bounds_checked() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.u16().unwrap(), 0x0201);
        assert!(r.u32().is_err());
    }
}
