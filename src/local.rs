//! References to originals selected by the user. Image bytes stay in their
//! original files. Each preference record keeps its path and favourite together.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_REFS: usize = 12;
pub const LIBRARY_KEY: &str = "linked-originals";

#[derive(Deserialize, Serialize)]
pub struct Reference {
    pub path: PathBuf,
    pub favourite: bool,
}

pub fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

pub fn read(saved: Option<Vec<Reference>>) -> Vec<Reference> {
    let mut originals = Vec::<Reference>::new();
    for record in saved.unwrap_or_default() {
        if record.path.is_absolute() && !originals.iter().any(|item| item.path == record.path) {
            originals.push(record);
            if originals.len() == MAX_REFS {
                break;
            }
        }
    }
    originals
}
