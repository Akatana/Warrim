//! Minimal MPQ v1 reader: enough to serve every file the WoW 1.12 client loads.
//!
//! Supported: hash/block tables, encryption, zlib / bzip2 / PKWARE compression.
//! Not supported (audio only): Huffman, ADPCM, sparse compression.

mod archive;
mod compress;
pub mod crypt;
mod mount;

pub use archive::{Archive, BlockEntry, FLAG_DELETE_MARKER, FLAG_EXISTS};
pub use mount::{find_wow_dir, Mount};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt archive: {0}")]
    Corrupt(&'static str),
    #[error("unsupported feature: {0}")]
    Unsupported(&'static str),
    #[error("file not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;
