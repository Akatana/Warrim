//! Parsers for the file formats of the WoW 1.12.x client (build 5875).

pub mod adt;
pub mod blp;
pub mod dbc;
pub mod m2;
pub mod reader;
pub mod wmo;
pub mod wdt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unexpected end of data (wanted {wanted} bytes at offset {at}, have {len})")]
    Eof { at: usize, wanted: usize, len: usize },
    #[error("bad magic: expected {expected}, found {found:?}")]
    Magic { expected: &'static str, found: String },
    #[error("invalid data: {0}")]
    Invalid(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;
