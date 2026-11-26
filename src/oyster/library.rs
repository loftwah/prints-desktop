//! Library-focused typed methods for [`super::OysterClient`].
//!
//! This child module exposes only reads: `listFolders` for enumeration and
//! `getOutput` for metadata verification around an original-byte download.
//! No mutation APIs live here.

#![forbid(unsafe_code)]

use std::sync::atomic::AtomicBool;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Error, OutputItem, OysterClient, TransferProgress};

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

/// An immutable capture of the exact output an original transfer is bound to.
/// The visible page and selection may move on while the transfer runs; this
/// value, including its locally computed seal, never changes.
#[derive(Debug, Clone)]
pub struct TransferCapture {
    item: OutputItem,
    seal: u64,
}

impl TransferCapture {
    /// Bind one selected output to a connection generation and a request id.
    pub fn new(item: &OutputItem, generation: u64, correlation: &str) -> Self {
        let seal = seal_transfer(item, generation, correlation);
        Self {
            item: item.clone(),
            seal,
        }
    }

    /// An opaque, local seal over the whole identity. It never includes a key,
    /// host, prompt or file path in any report.
    pub fn seal(&self) -> u64 {
        self.seal
    }

    /// Whether a reply can be matched back to the immutable capture.
    pub fn is_sealed_by(&self, generation: u64, correlation: &str) -> bool {
        seal_transfer(&self.item, generation, correlation) == self.seal
    }

    /// Only an explicit, positive record size is a trustworthy total.
    pub fn total(&self) -> Option<u64> {
        (self.item.print.bytes > 0).then_some(self.item.print.bytes)
    }

    pub fn path(&self) -> &str {
        &self.item.path
    }

    pub fn item(&self) -> &OutputItem {
        &self.item
    }
}

/// A small deterministic seal, not a secret and never sent upstream. It binds
/// the selected output, generation and request id so a result from another
/// selection or connection cannot be mistaken for this transfer's answer.
fn seal_transfer(item: &OutputItem, generation: u64, correlation: &str) -> u64 {
    let print = &item.print;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    mix(&mut hash, &generation.to_be_bytes());
    mix(&mut hash, correlation.as_bytes());
    mix(&mut hash, item.path.as_bytes());
    mix(&mut hash, print.print_id.as_bytes());
    mix(&mut hash, print.computer_id.as_bytes());
    mix(&mut hash, &print.bytes.to_be_bytes());
    mix(&mut hash, &print.created_at.to_be_bytes());
    hash
}

fn mix(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    *hash ^= 0xa5;
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
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

    /// A cancellable transfer of one captured original. Metadata is verified
    /// before and after the byte reads, exactly as in `verify_and_download`.
    /// `cancel` is consulted only at chunk boundaries; a request already sent
    /// is allowed to finish and its bytes are dropped. Progress is reported
    /// only after bytes are decoded, and the total is passed through only when
    /// the capture carried an explicit size.
    pub fn transfer_original(
        &self,
        capture: &TransferCapture,
        cancel: &AtomicBool,
        mut on_progress: impl FnMut(TransferProgress),
    ) -> Result<Vec<u8>, Error> {
        let expected = capture.item();
        if !expected.path.starts_with('/') {
            return Err(Error::Decode);
        }
        let before = self.get_output(&expected.path)?;
        same_output(expected, &before)?;
        let bytes = self.download_observed(
            &expected.print.print_id,
            Some(&expected.path),
            capture.total(),
            Some(cancel),
            Some(&mut on_progress),
        )?;
        let after = self.get_output(&expected.path)?;
        same_output(expected, &after)?;
        if let Some(total) = capture.total()
            && bytes.len() as u64 != total
        {
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

    #[test]
    fn transfer_capture_seals_immutable_identity_and_trustworthy_total() {
        let capture = TransferCapture::new(&item(), 7, "op-3");
        assert_eq!(
            capture.seal(),
            TransferCapture::new(&item(), 7, "op-3").seal()
        );
        assert!(capture.is_sealed_by(7, "op-3"));
        assert!(!capture.is_sealed_by(8, "op-3"));
        assert!(!capture.is_sealed_by(7, "op-4"));
        assert_eq!(capture.total(), Some(3));

        let mut other = item();
        other.print.print_id = "print_other".into();
        assert_ne!(
            TransferCapture::new(&other, 7, "op-3").seal(),
            capture.seal()
        );
        let mut other = item();
        other.path = "/prints/other.png".into();
        assert_ne!(
            TransferCapture::new(&other, 7, "op-3").seal(),
            capture.seal()
        );

        let mut unknown = item();
        unknown.print.bytes = 0;
        assert_eq!(TransferCapture::new(&unknown, 7, "op-3").total(), None);
    }

    #[test]
    fn cancel_stops_chunk_dispatch_and_drops_buffered_bytes() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::thread;

        use super::super::ApiKey;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let dispatched = Arc::new(AtomicUsize::new(0));
        let count = dispatched.clone();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = BufReader::new(&mut socket);
            let mut line = String::new();
            request.read_line(&mut line).unwrap();
            assert!(line.starts_with("POST /apps/prints/methods/openOutput "));
            let mut length = 0;
            loop {
                line.clear();
                request.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.trim().split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            request.read_exact(&mut body).unwrap();
            let payload = serde_json::to_vec(&json!({
                "printId": "print_abc", "path": "/prints/print.png", "byteOffset": 0,
                "content": "AAECAw==", "encoding": "base64", "truncated": true,
            }))
            .unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len()).unwrap();
            socket.write_all(&payload).unwrap();
            count.fetch_add(1, Ordering::SeqCst);
        });
        let client =
            OysterClient::new(&origin, "prints", ApiKey::new("oyk_local_fixture_123456")).unwrap();
        let mut observed = item();
        observed.print.bytes = 4;
        let capture = TransferCapture::new(&observed, 1, "op-1");
        let mut progress = Vec::new();
        let stop = cancel.clone();
        let result = client.download_observed(
            &observed.print.print_id,
            Some(&observed.path),
            capture.total(),
            Some(&cancel),
            Some(&mut |update: TransferProgress| {
                progress.push(update);
                // A stop after one observed chunk must stop the next dispatch.
                stop.store(true, Ordering::Release);
            }),
        );
        assert_eq!(result, Err(Error::Cancelled));
        assert_eq!(
            dispatched.load(Ordering::SeqCst),
            1,
            "stop must prevent a later request"
        );
        assert_eq!(
            progress,
            vec![TransferProgress {
                observed: 4,
                total: Some(4)
            }]
        );
        server.join().unwrap();
    }
}
