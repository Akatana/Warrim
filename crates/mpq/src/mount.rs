//! A set of archives layered like the WoW client does: later patches shadow
//! earlier archives, and a delete marker in a patch hides the file entirely.

use crate::{Archive, Error, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub struct Mount {
    /// Highest priority first.
    archives: Vec<Archive>,
}

/// Lower number = higher priority.
fn priority(file_name: &str) -> (u32, String) {
    let lower = file_name.to_ascii_lowercase();
    let stem = lower.trim_end_matches(".mpq");
    let rank = if let Some(n) = stem.strip_prefix("patch-") {
        // patch-9 beats patch-2 beats patch (rank 20).
        10 - n.parse::<u32>().unwrap_or(0).min(9)
    } else if stem == "patch" {
        20
    } else if stem.starts_with("patch") || stem.starts_with("locale") || stem.starts_with("speech") {
        30
    } else if stem == "backup" {
        90
    } else if stem == "base" {
        80
    } else {
        50
    };
    (rank, lower)
}

/// Locate a WoW install: `$WOW_DIR`, else the current directory or any parent
/// (and the executable's parents) that contains `Data/dbc.MPQ`.
pub fn find_wow_dir() -> Option<PathBuf> {
    let looks_right = |p: &Path| p.join("Data").join("dbc.MPQ").exists() || p.join("Data").join("dbc.mpq").exists();
    if let Some(d) = std::env::var_os("WOW_DIR") {
        let d = PathBuf::from(d);
        return looks_right(&d).then_some(d);
    }
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok()];
    starts
        .into_iter()
        .flatten()
        .find_map(|s| s.ancestors().find(|a| looks_right(a)).map(Path::to_path_buf))
}

impl Mount {
    /// Mount every `*.mpq` in `<wow_dir>/Data`.
    pub fn open_wow_dir(wow_dir: impl AsRef<Path>) -> Result<Mount> {
        let data = wow_dir.as_ref().join("Data");
        let mut files: Vec<(PathBuf, String)> = std::fs::read_dir(&data)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.to_ascii_lowercase().ends_with(".mpq").then(|| (e.path(), name))
            })
            .collect();
        if files.is_empty() {
            return Err(Error::Corrupt("no .mpq files found in Data directory"));
        }
        files.sort_by_key(|(_, n)| priority(n));
        Self::open_paths(files.into_iter().map(|(p, _)| p))
    }

    /// Mount explicit archives, highest priority first.
    pub fn open_paths(paths: impl IntoIterator<Item = PathBuf>) -> Result<Mount> {
        let mut archives = Vec::new();
        for p in paths {
            match Archive::open(&p) {
                Ok(a) => {
                    log::info!("mounted {} ({} blocks)", p.display(), a.file_count());
                    archives.push(a);
                }
                Err(e) => log::warn!("skipping {}: {e}", p.display()),
            }
        }
        Ok(Mount { archives })
    }

    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        for a in &self.archives {
            if a.is_deleted(name) {
                break;
            }
            if let Some(data) = a.read(name)? {
                return Ok(data);
            }
        }
        Err(Error::NotFound(name.to_string()))
    }

    pub fn exists(&self, name: &str) -> bool {
        for a in &self.archives {
            if a.is_deleted(name) {
                return false;
            }
            if a.contains(name) {
                return true;
            }
        }
        false
    }

    /// Which archive would serve `name`.
    pub fn locate(&self, name: &str) -> Option<&Path> {
        self.archives
            .iter()
            .take_while(|a| !a.is_deleted(name))
            .find(|a| a.contains(name))
            .map(|a| a.path())
    }

    /// Union of every `(listfile)`; sorted, de-duplicated.
    pub fn list(&self) -> Vec<String> {
        let set: BTreeSet<String> = self.archives.iter().flat_map(|a| a.listfile()).collect();
        set.into_iter().collect()
    }
}
