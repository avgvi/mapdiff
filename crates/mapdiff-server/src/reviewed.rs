//! Which changes have been marked as reviewed.
//!
//! Marks are kept in the repository's `.git` directory, so they belong to
//! this clone, survive restarts, and can never be committed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    keys: BTreeSet<String>,
}

pub struct ReviewedStore {
    file: PathBuf,
}

impl ReviewedStore {
    pub fn new(git_dir: &Path) -> Self {
        Self {
            file: git_dir.join("mapdiff").join("reviewed.json"),
        }
    }

    /// Every stored mark, sorted. A missing or unreadable file means none.
    pub fn load(&self) -> BTreeSet<String> {
        std::fs::read(&self.file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
            .unwrap_or_default()
            .keys
    }

    /// Adds or removes marks and returns the full set afterwards.
    pub fn update(&self, keys: &[String], reviewed: bool) -> std::io::Result<BTreeSet<String>> {
        let mut all = self.load();
        for key in keys {
            if reviewed {
                all.insert(key.clone());
            } else {
                all.remove(key);
            }
        }
        if let Some(directory) = self.file.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let stored = Stored { keys: all };
        let text = serde_json::to_vec_pretty(&stored).map_err(std::io::Error::other)?;
        // Write beside the file, then swap, so a crash never leaves it half written.
        let temporary = self.file.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, &self.file)?;
        Ok(stored.keys)
    }
}
