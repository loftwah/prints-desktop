//! Library-focused typed methods for [`super::OysterClient`].
//!
//! This child module exposes only reads: `listFolders` for enumeration and
//! `getOutput` for metadata verification around an original-byte download.
//! No mutation APIs live here.

#![forbid(unsafe_code)]

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Error, OutputItem, OysterClient};

/// Largest page `listFolders` accepts (`methods.ts:489`: `min(1).max(100)`).
const MAX_FOLDER_PAGE: u8 = 100;
/// JSON numbers above 2^53-1 cannot be represented exactly by the upstream
/// (`methods.ts:484-487`), so never send a larger `after` offset.
const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;

/// One sub-folder returned by `listFolders` (`methods.ts:493-499`).
#[derive(Debug, Clone, Deserialize)]
pub struct Folder {
    pub name: String,
    pub folder: String,
}

/// One page of `listFolders`; `next` is the offset to pass back as `after`.
#[derive(Debug, Clone, Deserialize)]
pub struct FolderPage {
    pub folders: Vec<Folder>,
    pub next: Option<u64>,
}

impl OysterClient {
    /// `listFolders` — requires approved `computer.list`. Pass a relative
    /// library path as `folder` (omit for the library folder itself) and the
    /// `next` offset from a prior page as `after`. Pages hold at most 100
    /// entries.
    pub fn library_folders(
        &self,
        folder: Option<&str>,
        after: Option<u64>,
    ) -> Result<FolderPage, Error> {
        if let Some(folder) = folder {
            if !super::valid_folder(folder) {
                return Err(Error::InvalidFolder);
            }
        }
        if after.is_some_and(|n| n > MAX_SAFE_INTEGER) {
            return Err(Error::InvalidArgument);
        }
        let mut input = json!({ "limit": MAX_FOLDER_PAGE });
        if let Some(folder) = folder {
            input["folder"] = Value::String(folder.to_owned());
        }
        if let Some(after) = after {
            input["after"] = json!(after);
        }
        let value = self.call("listFolders", input)?;
        serde_json::from_value(value).map_err(|_| Error::Decode)
    }

    /// Verify an output's metadata with `getOutput`, download its original
    /// bytes, then verify the metadata again. The expected item and both
    /// `getOutput` replies must agree on path, print identity, computer ID,
    /// size and date, and the assembled bytes must be exactly
    /// `expected.print.bytes` long. `getOutput` needs approved `computer.list`;
    /// the download needs approved `computer.readFile`.
    pub fn verify_and_download(&self, expected: &OutputItem) -> Result<Vec<u8>, Error> {
        if !expected.path.starts_with('/') {
            return Err(Error::Decode);
        }
        let before = self.get_output(&expected.path)?;
        same_output(expected, &before)?;
        let bytes =
            self.download_matching_output(&expected.print.print_id, Some(&expected.path))?;
        let after = self.get_output(&expected.path)?;
        same_output(expected, &after)?;
        if bytes.len() as u64 != expected.print.bytes {
            return Err(Error::Decode);
        }
        Ok(bytes)
    }

    fn get_output(&self, path: &str) -> Result<OutputItem, Error> {
        if !path.starts_with('/') {
            return Err(Error::Decode);
        }
        let value = self.call("getOutput", json!({ "path": path }))?;
        serde_json::from_value(value).map_err(|_| Error::Decode)
    }
}

/// `getOutput` must describe the very same output the caller selected.
fn same_output(expected: &OutputItem, actual: &OutputItem) -> Result<(), Error> {
    let e = &expected.print;
    let a = &actual.print;
    if expected.path != actual.path
        || e.print_id != a.print_id
        || e.computer_id != a.computer_id
        || e.bytes != a.bytes
        || e.created_at != a.created_at
    {
        return Err(Error::Decode);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> OutputItem {
        serde_json::from_value(json!({
            "print": {
                "printId": "print_abc",
                "computerId": "computer-1",
                "kind": "image",
                "model": "flux",
                "format": "png",
                "bytes": 3,
                "createdAt": 1730000000000u64,
            },
            "path": "/prints/print.png",
        }))
        .unwrap()
    }

    #[test]
    fn folder_page_parses() {
        let page: FolderPage = serde_json::from_value(json!({
            "folders": [{"name": "Mira", "folder": "Characters/Mira"}],
            "next": 100,
        }))
        .unwrap();
        assert_eq!(page.folders[0].folder, "Characters/Mira");
        assert_eq!(page.next, Some(100));
    }

    #[test]
    fn get_output_extra_fields_are_ignored() {
        let value = json!({
            "print": {
                "printId": "print_abc",
                "computerId": "computer-1",
                "kind": "image",
                "model": "flux",
                "format": "png",
                "bytes": 3,
                "createdAt": 1730000000000u64,
            },
            "path": "/prints/print.png",
            "references": [],
            "characters": [],
            "style": null,
            "typed": {"prompt": "An original composition.", "references": 0},
        });
        let parsed: OutputItem = serde_json::from_value(value).unwrap();
        same_output(&item(), &parsed).unwrap();
    }

    #[test]
    fn metadata_mismatch_is_rejected() {
        let mut other = item();
        other.print.bytes += 1;
        assert_eq!(same_output(&item(), &other), Err(Error::Decode));
        let mut other = item();
        other.print.created_at += 1;
        assert_eq!(same_output(&item(), &other), Err(Error::Decode));
        let mut other = item();
        other.path = "/prints/other.png".to_owned();
        assert_eq!(same_output(&item(), &other), Err(Error::Decode));
    }
}
