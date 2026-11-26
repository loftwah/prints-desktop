//! References to originals selected by the user. Image bytes stay in their
//! original files. Each preference record keeps its path and favourite together.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{connected_asset::Provenance, workspace::AssetId};

pub const MAX_REFS: usize = 300;
pub const LIBRARY_KEY: &str = "linked-originals";

#[derive(Deserialize, Serialize)]
pub struct Reference {
    /// Stable, path-free identity. Old snapshots have no id and are migrated
    /// once, then kept across reorder, reopen and relocation.
    #[serde(default)]
    pub id: AssetId,
    /// Identity of a hosted original this local file was deliberately copied
    /// from, plus a digest to distinguish a later replacement at the same path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(with = "native_path")]
    pub path: PathBuf,
    pub favourite: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_draft: bool,
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

/// Give legacy references an opaque id before they are saved again. Ids are
/// path-free, unique inside this library, and never derived from a filename.
pub fn migrate_ids(records: &mut [Reference]) -> bool {
    let mut used: HashSet<String> = records
        .iter()
        .filter(|record| !record.id.is_empty())
        .map(|record| record.id.as_str().to_owned())
        .collect();
    let mut changed = false;
    for (index, record) in records.iter_mut().enumerate() {
        if record.id.is_empty() {
            record.id = AssetId::migrated(&used, used.len() + index);
            used.insert(record.id.as_str().to_owned());
            changed = true;
        }
    }
    changed
}

// Keep the existing UTF-8 representation and preserve other legal paths
// through serde's native OsStr encoding on the same operating system.
pub(crate) mod native_path {
    use std::ffi::OsString;

    use super::*;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Text(PathBuf),
        Native(OsString),
    }

    pub(crate) fn serialize<S: serde::Serializer>(
        path: &Path,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match path.to_str() {
            Some(text) => text.serialize(serializer),
            None => path.as_os_str().serialize(serializer),
        }
    }

    pub(crate) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PathBuf, D::Error> {
        match Stored::deserialize(deserializer)? {
            Stored::Text(path) => Ok(path),
            Stored::Native(path) => Ok(path.into()),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    #[test]
    fn linked_original_paths_round_trip_without_losing_native_bytes() {
        let old: Reference =
            serde_json::from_str(r#"{"path":"/first.jpg","favourite":true}"#).unwrap();
        assert_eq!(old.path, Path::new("/first.jpg"));
        assert!(old.favourite);
        assert!(!old.in_draft);
        assert!(old.id.is_empty());
        assert!(old.provenance.is_none());

        let reference = Reference {
            id: AssetId::bundled("archive", "one.jpg"),
            provenance: None,
            path: OsString::from_vec(b"/x\xff.jpg".to_vec()).into(),
            favourite: true,
            in_draft: true,
        };
        let encoded = serde_json::to_string(&reference).unwrap();
        assert!(!encoded.contains("provenance"));
        let decoded: Reference = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.path, reference.path);
        assert_eq!(decoded.id, reference.id);
        assert!(decoded.favourite);
        assert!(decoded.in_draft);
        assert!(decoded.provenance.is_none());
    }

    #[test]
    fn legacy_ids_are_migrated_once_and_survive_reorder_and_relocation() {
        let mut records = vec![
            Reference {
                id: AssetId::default(),
                provenance: None,
                path: PathBuf::from("/first.jpg"),
                favourite: false,
                in_draft: true,
            },
            Reference {
                id: AssetId::default(),
                provenance: None,
                path: PathBuf::from("/second.jpg"),
                favourite: true,
                in_draft: false,
            },
        ];
        assert!(migrate_ids(&mut records));
        let first = records[0].id.clone();
        let second = records[1].id.clone();
        assert_ne!(first, second);
        assert!(!first.as_str().contains("first.jpg"));
        assert!(!second.as_str().contains("second.jpg"));
        assert!(!migrate_ids(&mut records));

        records.swap(0, 1);
        records[1].path = PathBuf::from("/relocated.jpg");
        assert_eq!(records[1].id, first);
        assert_eq!(records[0].id, second);
    }
}
