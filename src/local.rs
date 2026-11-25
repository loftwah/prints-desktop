//! References to originals selected by the user. Image bytes stay in their
//! original files. Each preference record keeps its path and favourite together.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_REFS: usize = 300;
pub const LIBRARY_KEY: &str = "linked-originals";

#[derive(Deserialize, Serialize)]
pub struct Reference {
    #[serde(with = "native_path")]
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

// Keep the existing UTF-8 representation and preserve other legal paths
// through serde's native OsStr encoding on the same operating system.
mod native_path {
    use std::ffi::OsString;

    use super::*;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Text(PathBuf),
        Native(OsString),
    }

    pub(super) fn serialize<S: serde::Serializer>(
        path: &Path,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match path.to_str() {
            Some(text) => text.serialize(serializer),
            None => path.as_os_str().serialize(serializer),
        }
    }

    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
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

        let reference = Reference {
            path: OsString::from_vec(b"/x\xff.jpg".to_vec()).into(),
            favourite: true,
        };
        let encoded = serde_json::to_string(&reference).unwrap();
        let decoded: Reference = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.path, reference.path);
        assert!(decoded.favourite);
    }
}
