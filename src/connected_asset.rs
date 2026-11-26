use serde::{Deserialize, Serialize};

use crate::{AssetId, WorkspaceId};
use prints_desktop::oyster::{OutputItem, OysterClient};

/// The upstream print record survives a move. Its path and any displayed
/// metadata are session data, so a saved reference carries neither.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Identity {
    origin: String,
    installation: String,
    computer: String,
    output: String,
    created_at: u64,
}

impl Identity {
    pub(crate) fn new(client: &OysterClient, item: &OutputItem) -> Self {
        let (origin, installation) = client.identity();
        Self {
            origin,
            installation,
            computer: item.print.computer_id.clone(),
            output: item.print.print_id.clone(),
            created_at: item.print.created_at,
        }
    }

    pub(crate) fn label(&self) -> String {
        format!("{} · {}", self.origin, self.installation)
    }

    pub(crate) fn id(&self, workspace: &WorkspaceId) -> AssetId {
        let mut hash = blake3::Hasher::new();
        hash.update(b"prints-hosted-reference-v1");
        for field in [
            workspace.as_str(),
            &self.origin,
            &self.installation,
            &self.computer,
            &self.output,
        ] {
            hash.update(&(field.len() as u64).to_be_bytes());
            hash.update(field.as_bytes());
        }
        hash.update(&self.created_at.to_be_bytes());
        AssetId::hosted(hash.finalize().to_hex().as_str())
    }
}

/// The source link belongs to the exported bytes, even after the key is gone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Provenance {
    pub source: Identity,
    content_digest: String,
}

impl Provenance {
    pub fn new(source: Identity, bytes: &[u8]) -> Self {
        Self {
            source,
            content_digest: blake3::hash(bytes).to_hex().to_string(),
        }
    }

    pub fn matches(&self, digest: &blake3::Hash) -> bool {
        self.content_digest == digest.to_hex().as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prints_desktop::oyster::ApiKey;

    fn item(path: &str, computer: &str, created_at: u64) -> OutputItem {
        serde_json::from_value(serde_json::json!({
            "path": path,
            "print": {
                "printId": "print_same", "computerId": computer, "kind": "image",
                "model": "fixture", "format": "jpeg", "bytes": 12, "createdAt": created_at
            }
        }))
        .unwrap()
    }

    #[test]
    fn a_move_keeps_identity_but_a_replacement_cannot_retarget_it() {
        let client = |origin, install| {
            OysterClient::new(origin, install, ApiKey::new("oyk_fictional_scoped_fixture")).unwrap()
        };
        let host = client("https://oyster.example", "prints");
        let original = Identity::new(&host, &item("/old.jpg", "first", 1));
        let moved = Identity::new(&host, &item("/new.jpg", "first", 1));
        assert_eq!(original, moved);
        let workspace = WorkspaceId::current();
        assert_eq!(original.id(&workspace), moved.id(&workspace));
        for replacement in [
            Identity::new(&host, &item("/new.jpg", "other", 1)),
            Identity::new(&host, &item("/new.jpg", "first", 2)),
            Identity::new(
                &client("https://another.example", "prints"),
                &item("/old.jpg", "first", 1),
            ),
            Identity::new(
                &client("https://oyster.example", "another"),
                &item("/old.jpg", "first", 1),
            ),
        ] {
            assert_ne!(original.id(&workspace), replacement.id(&workspace));
        }
        let stored = serde_json::to_string(&original).unwrap();
        assert!(!stored.contains(".jpg"));
        assert!(!stored.contains("fixture"));
        let copy = Provenance::new(original, b"the exact original");
        assert!(copy.matches(&blake3::hash(b"the exact original")));
        assert!(!copy.matches(&blake3::hash(b"a different file")));
    }
}
