//! WDBC database tables (`DBFilesClient\*.dbc`).

use crate::reader::Reader;
use crate::{Error, Result};

pub struct Dbc {
    pub record_count: usize,
    pub field_count: usize,
    pub record_size: usize,
    records: Vec<u8>,
    strings: Vec<u8>,
}

impl Dbc {
    pub fn parse(data: &[u8]) -> Result<Dbc> {
        let mut r = Reader::new(data);
        let magic = r.bytes(4)?;
        if magic != b"WDBC" {
            return Err(Error::Magic { expected: "WDBC", found: String::from_utf8_lossy(magic).into() });
        }
        let record_count = r.u32()? as usize;
        let field_count = r.u32()? as usize;
        let record_size = r.u32()? as usize;
        let string_size = r.u32()? as usize;
        let records = r.bytes(record_count * record_size)?.to_vec();
        let strings = r.bytes(string_size)?.to_vec();
        Ok(Dbc { record_count, field_count, record_size, records, strings })
    }

    pub fn record(&self, index: usize) -> Option<Record<'_>> {
        (index < self.record_count).then(|| Record {
            dbc: self,
            data: &self.records[index * self.record_size..(index + 1) * self.record_size],
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = Record<'_>> {
        (0..self.record_count).filter_map(|i| self.record(i))
    }

    /// First record whose field 0 (the primary key in every table we care about) equals `id`.
    pub fn find_by_id(&self, id: u32) -> Option<Record<'_>> {
        self.iter().find(|r| r.u32(0) == id)
    }

    pub fn string_at(&self, offset: u32) -> &str {
        let s = self.strings.get(offset as usize..).unwrap_or(&[]);
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        std::str::from_utf8(&s[..end]).unwrap_or("")
    }
}

#[derive(Clone, Copy)]
pub struct Record<'a> {
    dbc: &'a Dbc,
    data: &'a [u8],
}

impl<'a> Record<'a> {
    pub fn u32(&self, field: usize) -> u32 {
        let o = field * 4;
        self.data
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0)
    }
    pub fn i32(&self, field: usize) -> i32 {
        self.u32(field) as i32
    }
    pub fn f32(&self, field: usize) -> f32 {
        f32::from_bits(self.u32(field))
    }
    /// A string field is an offset into the table's string block.
    pub fn string(&self, field: usize) -> &'a str {
        self.dbc.string_at(self.u32(field))
    }
}
