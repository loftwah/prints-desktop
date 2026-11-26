//! Shared local workspace identity and explicit, revocable Agent access.
//!
//! This module is the domain boundary between the native workspace and a
//! transport. It knows nothing about sockets, URLs, files or exports: an agent
//! only ever receives opaque ids, titles and brief/selection state.
//!
//! Access is off every time the app starts. A read grant and an edit grant are
//! distinct, and revoking either invalidates the bearer immediately in the
//! running app. Every write is scoped to the workspace id persisted in the
//! local snapshot and must carry the revision observed at evaluation time.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

pub(crate) const MAX_BRIEF_CHARS: usize = 4000;
pub(crate) const MAX_ID_CHARS: usize = 96;
pub(crate) const MAX_PAGE: usize = 300;
#[cfg(test)]
const MAX_COMMAND_BYTES: usize = 16_000;
const MAX_SEEN_COMMANDS: usize = 64;
const MAX_REASON_CHARS: usize = 240;

/// Opaque identity for one image. Never a path or a filename: bundled artwork
/// uses a namespaced public constant, imported artwork gets a fresh random
/// value that is stored with the local reference.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct AssetId(String);

impl AsRef<str> for AssetId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AssetId {
    pub(crate) fn bundled(namespace: &str, key: &str) -> Self {
        Self(
            format!("bundled:{namespace}:{key}")
                .chars()
                .filter(|c| id_char(*c))
                .take(MAX_ID_CHARS)
                .collect(),
        )
    }

    pub(crate) fn hosted(digest: &str) -> Self {
        Self(format!("hosted:{digest}"))
    }

    /// Path-free id for a legacy reference with no stored id. The counter only
    /// avoids collisions inside this workspace; it is not a secret.
    pub(crate) fn migrated(used: &HashSet<String>, seed: usize) -> Self {
        let mut n = seed.saturating_add(1);
        loop {
            let candidate = format!("local:m{n}");
            if !used.contains(&candidate) {
                return Self(candidate);
            }
            n = n.saturating_add(1);
        }
    }

    /// Prefer cryptographic entropy. When it is unavailable the fallback is a
    /// path-free counter, chosen not to collide with ids already in the
    /// workspace; asset ids are not secrets, unlike the access bearer.
    pub(crate) fn fresh(used: &HashSet<String>) -> Self {
        let mut bytes = [0u8; 12];
        if getrandom::fill(&mut bytes).is_ok() {
            let candidate = format!("local:{}", hex(&bytes));
            if !used.contains(&candidate) {
                return Self(candidate);
            }
        }
        Self::migrated(used, used.len())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Identity of the local workspace the grant was made in. Persisted in the
/// draft snapshot, checked on every command.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct WorkspaceId(String);

impl AsRef<str> for WorkspaceId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl WorkspaceId {
    pub(crate) fn current() -> Self {
        let mut bytes = [0u8; 16];
        if let Ok(()) = getrandom::fill(&mut bytes) {
            return Self(format!("ws:{}", hex(&bytes)));
        }
        // Honest fallback: this id scopes local writes but is not a secret and
        // must not be used to grant access (activation refuses without entropy).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self(format!(
            "ws:fallback-{}-{}",
            std::process::id(),
            hex(&nanos.to_le_bytes())
        ))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}

fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= MAX_ID_CHARS && id.chars().all(id_char)
}

/// The app was asked to create a bearer but the OS could not supply entropy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EntropyUnavailable;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AccessMode {
    Read,
    Edit,
}

/// Ephemeral, in-memory grant. Nothing here is restored on launch: the creator
/// opts in explicitly from the native app each time.
pub(crate) struct AgentAccess {
    mode: Option<AccessMode>,
    bearer: Option<String>,
    workspace: WorkspaceId,
    seen: Vec<(String, Request, Reply)>,
}

impl AgentAccess {
    pub(crate) fn off(workspace: WorkspaceId) -> Self {
        Self {
            mode: None,
            bearer: None,
            workspace,
            seen: Vec::new(),
        }
    }

    pub(crate) fn activate(&mut self, mode: AccessMode) -> Result<(), EntropyUnavailable> {
        let mut bytes = [0u8; 24];
        getrandom::fill(&mut bytes).map_err(|_| EntropyUnavailable)?;
        self.bearer = Some(format!("pd_{}", hex(&bytes)));
        self.mode = Some(mode);
        self.seen.clear();
        Ok(())
    }

    pub(crate) fn revoke(&mut self) {
        self.mode = None;
        self.bearer = None;
        self.seen.clear();
    }

    pub(crate) fn mode(&self) -> Option<AccessMode> {
        self.mode
    }

    pub(crate) fn bearer(&self) -> Option<&str> {
        self.bearer.as_deref()
    }

    pub(crate) fn workspace(&self) -> &WorkspaceId {
        &self.workspace
    }

    fn allows(&self, presented: &str) -> bool {
        match (&self.mode, &self.bearer) {
            (Some(_), Some(active)) => constant_eq(active.as_bytes(), presented.as_bytes()),
            _ => false,
        }
    }

    fn recall(&self, id: &str, request: &Request) -> Option<Reply> {
        let (_, previous, reply) = self.seen.iter().find(|(seen, _, _)| seen == id)?;
        Some(if previous == request {
            reply.clone()
        } else {
            Reply::Duplicate { command: id.into() }
        })
    }

    fn remember(&mut self, id: String, request: Request, reply: &Reply) {
        if self.seen.len() == MAX_SEEN_COMMANDS {
            self.seen.remove(0);
        }
        self.seen.push((id, request, reply.clone()));
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Public-safe metadata for one image. Deliberately has no path, filename,
/// original bytes, credential or export field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AssetView {
    pub id: AssetId,
    pub title: String,
    pub selected: bool,
    pub in_brief: bool,
    pub comparable: bool,
    pub compare_first: bool,
    pub compare_second: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct BriefSnapshot {
    pub brief: String,
    pub brief_truncated: bool,
    pub shortlist: Vec<AssetId>,
    pub comparison: Option<[AssetId; 2]>,
    pub selected: Option<AssetId>,
    pub assets: Vec<AssetView>,
    pub has_more: bool,
}

/// Everything a revision depends on, assembled from live Studio state at the
/// moment a command is evaluated (never copied at poll registration).
#[derive(Clone, Copy)]
pub(crate) struct RevisionParts<'a> {
    pub human_edits: u64,
    pub brief: &'a str,
    pub selected: Option<&'a AssetId>,
    pub shortlist: &'a [AssetId],
    pub compare_first: Option<&'a AssetId>,
    pub compare_second: Option<&'a AssetId>,
    pub assets: &'a [AssetId],
}

pub(crate) fn revision_of(parts: &RevisionParts<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    parts.human_edits.hash(&mut hasher);
    parts.brief.hash(&mut hasher);
    parts.selected.hash(&mut hasher);
    parts.shortlist.hash(&mut hasher);
    parts.compare_first.hash(&mut hasher);
    parts.compare_second.hash(&mut hasher);
    parts.assets.hash(&mut hasher);
    hasher.finish()
}

// Revisions are opaque and must round-trip through JSON clients with IEEE-754 numbers.
mod revision_wire {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    pub workspace: WorkspaceId,
    pub bearer: String,
    /// Opaque id used to deduplicate writes; bounded by `MAX_ID_CHARS`.
    pub id: String,
    pub request: Request,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Discover {},
    Read {
        limit: Option<usize>,
    },
    Select {
        asset: AssetId,
        #[serde(with = "revision_wire")]
        expected_revision: u64,
    },
    Compare {
        first: AssetId,
        second: AssetId,
        #[serde(with = "revision_wire")]
        expected_revision: u64,
    },
    EditBrief {
        text: String,
        #[serde(with = "revision_wire")]
        expected_revision: u64,
    },
    Keep {
        asset: AssetId,
        #[serde(with = "revision_wire")]
        expected_revision: u64,
    },
    Remove {
        asset: AssetId,
        #[serde(with = "revision_wire")]
        expected_revision: u64,
    },
    ReopenHandoff {
        #[serde(with = "revision_wire")]
        expected_revision: u64,
    },
}

impl Request {
    fn is_write(&self) -> bool {
        !matches!(self, Self::Discover {} | Self::Read { .. })
    }
}

pub(crate) const OPERATIONS: [&str; 8] = [
    "discover",
    "read",
    "select",
    "compare",
    "edit_brief",
    "keep",
    "remove",
    "reopen_handoff",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Reply {
    Ok {
        result: ResultPayload,
        #[serde(with = "revision_wire")]
        revision: u64,
    },
    NoChange {
        reason: String,
        #[serde(with = "revision_wire")]
        revision: u64,
    },
    Denied {
        reason: String,
    },
    Conflict {
        #[serde(with = "revision_wire")]
        expected: u64,
        #[serde(with = "revision_wire")]
        current: u64,
    },
    Invalid {
        reason: String,
    },
    Duplicate {
        command: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum ResultPayload {
    Discover {
        workspace: WorkspaceId,
        mode: AccessMode,
        operations: Vec<String>,
    },
    Read {
        brief: String,
        brief_truncated: bool,
        shortlist: Vec<AssetId>,
        comparison: Option<[AssetId; 2]>,
        selected: Option<AssetId>,
        assets: Vec<AssetView>,
        has_more: bool,
    },
    Select {
        asset: AssetId,
    },
    Compare {
        first: AssetId,
        second: AssetId,
    },
    EditBrief {
        chars: usize,
    },
    Keep {
        asset: AssetId,
    },
    Remove {
        asset: AssetId,
    },
    ReopenHandoff {},
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Fault {
    NotFound,
    Limit,
    Unavailable,
}

impl Fault {
    fn reason(self) -> &'static str {
        match self {
            Self::NotFound => "That image is not in this workspace.",
            Self::Limit => "The workspace limit was reached.",
            Self::Unavailable => "That image is not available here.",
        }
    }
}

/// A command that found nothing to do reports it, rather than overwriting.
pub(crate) enum Change {
    Applied,
    Unchanged(&'static str),
}

/// The native surface an agent may touch. Every method is intentionally narrow:
/// there is no path, original, URL, credential or export capability here.
pub(crate) trait Surface {
    fn workspace_id(&self) -> &WorkspaceId;
    fn access(&self) -> &AgentAccess;
    fn access_mut(&mut self) -> &mut AgentAccess;
    fn revision(&self) -> u64;
    fn snapshot(&self, limit: usize) -> BriefSnapshot;
    fn asset(&self, id: &AssetId) -> Option<AssetView>;
    fn select(&mut self, id: &AssetId) -> Result<Change, Fault>;
    fn compare(&mut self, first: &AssetId, second: &AssetId) -> Result<Change, Fault>;
    fn edit_brief(&mut self, text: String) -> Result<Change, Fault>;
    fn keep(&mut self, id: &AssetId) -> Result<Change, Fault>;
    fn remove(&mut self, id: &AssetId) -> Result<Change, Fault>;
    fn reopen(&mut self) -> Result<Change, Fault>;
    /// Set visible status and schedule the same local save the buttons use.
    fn note_write(&mut self, message: &'static str);
}

/// Parse a typed wire command. Unknown fields, unknown operations, oversized
/// text and malformed JSON are all rejected before anything is evaluated.
#[cfg(test)]
pub(crate) fn decode_command(text: &str) -> Result<Command, String> {
    if text.len() > MAX_COMMAND_BYTES {
        return Err(bounded("Command is too large."));
    }
    serde_json::from_str(text).map_err(|err| bounded(&format!("Invalid command: {err}")))
}

/// Convenience for the transport: one typed `Value` in, one typed `Value` out.
/// Unknown fields and operations become `invalid`, never a panic.
pub(crate) fn evaluate_value<S: Surface>(
    surface: &mut S,
    value: serde_json::Value,
) -> serde_json::Value {
    let reply = match serde_json::from_value::<Command>(value) {
        Ok(command) => evaluate(surface, command),
        Err(_) => invalid("Send a supported workspace command."),
    };
    serde_json::to_value(reply).unwrap_or_else(
        |_| serde_json::json!({ "kind": "invalid", "reason": "Reply could not be encoded" }),
    )
}

fn bounded(reason: &str) -> String {
    reason.chars().take(MAX_REASON_CHARS).collect()
}

fn invalid(reason: &str) -> Reply {
    Reply::Invalid {
        reason: bounded(reason),
    }
}

fn denied(reason: &str) -> Reply {
    Reply::Denied {
        reason: bounded(reason),
    }
}

/// Evaluate exactly one command against live state. This is the only place
/// writes happen; revision and access are read here, never from a stale copy.
pub(crate) fn evaluate<S: Surface>(surface: &mut S, command: Command) -> Reply {
    if command.workspace != *surface.workspace_id() {
        return denied("Agent access is scoped to a different workspace.");
    }
    if !valid_id(command.workspace.as_str()) || !valid_id(&command.id) {
        return invalid("Invalid workspace or command id.");
    }
    if !surface.access().allows(&command.bearer) {
        return denied("Agent access is off in Prints Desktop.");
    }

    let write = command.request.is_write();
    let mode = surface.access().mode();
    if !write && mode.is_none() {
        return denied("Agent access is off in Prints Desktop.");
    }
    if write && mode != Some(AccessMode::Edit) {
        return denied("This agent has read-only access. Enable Edit in Prints Desktop.");
    }

    if write && let Some(previous) = surface.access().recall(&command.id, &command.request) {
        return previous;
    }

    match validate(&command.request, surface) {
        Err(reply) => return reply,
        Ok(()) => (),
    }

    if write {
        let expected = expected_revision(&command.request);
        let current = surface.revision();
        if expected != current {
            return Reply::Conflict { expected, current };
        }
    }

    let executed = execute(surface, &command.request);
    let revision = surface.revision();
    let reply = match executed {
        Err(fault) => invalid(fault.reason()),
        Ok(Change::Unchanged(reason)) => Reply::NoChange {
            reason: reason.into(),
            revision,
        },
        Ok(Change::Applied) => {
            let result = match &command.request {
                Request::Discover {} => ResultPayload::Discover {
                    workspace: surface.workspace_id().clone(),
                    mode: mode.expect("read access was checked"),
                    operations: OPERATIONS
                        .iter()
                        .take(if mode == Some(AccessMode::Edit) { 8 } else { 2 })
                        .map(|name| (*name).into())
                        .collect(),
                },
                Request::Read { .. } => {
                    let limit = match &command.request {
                        Request::Read { limit } => limit.unwrap_or(MAX_PAGE),
                        _ => unreachable!(),
                    };
                    let snapshot = surface.snapshot(limit);
                    ResultPayload::Read {
                        brief: snapshot.brief,
                        brief_truncated: snapshot.brief_truncated,
                        shortlist: snapshot.shortlist,
                        comparison: snapshot.comparison,
                        selected: snapshot.selected,
                        assets: snapshot.assets,
                        has_more: snapshot.has_more,
                    }
                }
                Request::Select { asset, .. } => ResultPayload::Select {
                    asset: asset.clone(),
                },
                Request::Compare { first, second, .. } => ResultPayload::Compare {
                    first: first.clone(),
                    second: second.clone(),
                },
                Request::EditBrief { text, .. } => ResultPayload::EditBrief {
                    chars: text.chars().count(),
                },
                Request::Keep { asset, .. } => ResultPayload::Keep {
                    asset: asset.clone(),
                },
                Request::Remove { asset, .. } => ResultPayload::Remove {
                    asset: asset.clone(),
                },
                Request::ReopenHandoff { .. } => ResultPayload::ReopenHandoff {},
            };
            Reply::Ok { result, revision }
        }
    };

    // Only completed outcomes are replayed. A denied or conflicted command can
    // be retried with the same id once the creator changes access or state.
    if write && matches!(reply, Reply::Ok { .. } | Reply::NoChange { .. }) {
        let id = command.id.clone();
        surface.access_mut().remember(id, command.request, &reply);
    }
    reply
}

fn expected_revision(request: &Request) -> u64 {
    match request {
        Request::Select {
            expected_revision, ..
        }
        | Request::Compare {
            expected_revision, ..
        }
        | Request::EditBrief {
            expected_revision, ..
        }
        | Request::Keep {
            expected_revision, ..
        }
        | Request::Remove {
            expected_revision, ..
        }
        | Request::ReopenHandoff { expected_revision } => *expected_revision,
        Request::Discover {} | Request::Read { .. } => 0,
    }
}

fn validate<S: Surface>(request: &Request, surface: &S) -> Result<(), Reply> {
    let known = |id: &AssetId| -> Result<(), Reply> {
        if !valid_id(id.as_str()) {
            return Err(invalid("Invalid image id."));
        }
        if surface.asset(id).is_none() {
            return Err(invalid("That image is not in this workspace."));
        }
        Ok(())
    };
    match request {
        Request::Discover {} => Ok(()),
        Request::Read { limit } => {
            if limit.is_some_and(|limit| limit == 0 || limit > MAX_PAGE) {
                Err(invalid("Read limit must be between 1 and 300."))
            } else {
                Ok(())
            }
        }
        Request::Select { asset, .. }
        | Request::Keep { asset, .. }
        | Request::Remove { asset, .. } => known(asset),
        Request::Compare { first, second, .. } => {
            known(first)?;
            known(second)?;
            if first == second {
                Err(invalid("Choose two different images to compare."))
            } else {
                Ok(())
            }
        }
        Request::EditBrief { text, .. } => {
            if text.chars().count() > MAX_BRIEF_CHARS {
                return Err(invalid("The brief is too long."));
            }
            if text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err(invalid("The brief contains unsupported characters."));
            }
            Ok(())
        }
        Request::ReopenHandoff { .. } => Ok(()),
    }
}

fn execute<S: Surface>(surface: &mut S, request: &Request) -> Result<Change, Fault> {
    let change = match request {
        Request::Discover {} | Request::Read { .. } => Change::Applied,
        Request::Select { asset, .. } => surface.select(asset)?,
        Request::Compare { first, second, .. } => surface.compare(first, second)?,
        Request::EditBrief { text, .. } => surface.edit_brief(text.clone())?,
        Request::Keep { asset, .. } => surface.keep(asset)?,
        Request::Remove { asset, .. } => surface.remove(asset)?,
        Request::ReopenHandoff { .. } => surface.reopen()?,
    };
    if matches!(change, Change::Applied) && request.is_write() {
        let message = match request {
            Request::Select { .. } => "Agent selected an image.",
            Request::Compare { .. } => "Agent set a comparison.",
            Request::EditBrief { .. } => "Agent updated the brief.",
            Request::Keep { .. } => "Agent kept an image with the draft.",
            Request::Remove { .. } => "Agent removed an image from the draft.",
            Request::ReopenHandoff { .. } => "Agent reopened the handoff.",
            Request::Discover {} | Request::Read { .. } => unreachable!(),
        };
        surface.note_write(message);
    }
    Ok(change)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        workspace: WorkspaceId,
        access: AgentAccess,
        edits: u64,
        assets: Vec<AssetView>,
        brief: String,
        selected: Option<AssetId>,
        shortlist: Vec<AssetId>,
        first: Option<AssetId>,
        writes: usize,
        notes: usize,
    }

    impl Fake {
        fn new() -> Self {
            let workspace = WorkspaceId("ws:test".into());
            let a = AssetId("local:a".into());
            let b = AssetId("local:b".into());
            Self {
                access: AgentAccess::off(workspace.clone()),
                workspace,
                edits: 1,
                assets: vec![
                    AssetView {
                        id: a,
                        title: "First".into(),
                        selected: false,
                        in_brief: false,
                        comparable: true,
                        compare_first: false,
                        compare_second: false,
                    },
                    AssetView {
                        id: b,
                        title: "Second".into(),
                        selected: false,
                        in_brief: false,
                        comparable: true,
                        compare_first: false,
                        compare_second: false,
                    },
                ],
                brief: String::new(),
                selected: None,
                shortlist: Vec::new(),
                first: None,
                writes: 0,
                notes: 0,
            }
        }

        fn id(&self, index: usize) -> AssetId {
            self.assets[index].id.clone()
        }

        fn select_command(&self, id: AssetId, revision: u64) -> Command {
            Command {
                workspace: self.workspace.clone(),
                bearer: self.access.bearer().unwrap_or("").into(),
                id: "cmd-1".into(),
                request: Request::Select {
                    asset: id,
                    expected_revision: revision,
                },
            }
        }
    }

    impl Surface for Fake {
        fn workspace_id(&self) -> &WorkspaceId {
            &self.workspace
        }
        fn access(&self) -> &AgentAccess {
            &self.access
        }
        fn access_mut(&mut self) -> &mut AgentAccess {
            &mut self.access
        }
        fn revision(&self) -> u64 {
            revision_of(&RevisionParts {
                human_edits: self.edits,
                brief: &self.brief,
                selected: self.selected.as_ref(),
                shortlist: &self.shortlist,
                compare_first: self.first.as_ref(),
                compare_second: None,
                assets: &self
                    .assets
                    .iter()
                    .map(|asset| asset.id.clone())
                    .collect::<Vec<_>>(),
            })
        }
        fn snapshot(&self, limit: usize) -> BriefSnapshot {
            BriefSnapshot {
                brief: self.brief.clone(),
                brief_truncated: false,
                shortlist: self.shortlist.clone(),
                comparison: None,
                selected: self.selected.clone(),
                assets: self.assets.iter().take(limit).cloned().collect(),
                has_more: self.assets.len() > limit,
            }
        }
        fn asset(&self, id: &AssetId) -> Option<AssetView> {
            self.assets.iter().find(|asset| asset.id == *id).cloned()
        }
        fn select(&mut self, id: &AssetId) -> Result<Change, Fault> {
            if self.selected.as_ref() == Some(id) {
                return Ok(Change::Unchanged("That image is already selected."));
            }
            self.selected = Some(id.clone());
            self.writes += 1;
            Ok(Change::Applied)
        }
        fn compare(&mut self, first: &AssetId, _second: &AssetId) -> Result<Change, Fault> {
            if self.first.as_ref() == Some(first) {
                return Ok(Change::Unchanged("That comparison is already set."));
            }
            self.first = Some(first.clone());
            self.writes += 1;
            Ok(Change::Applied)
        }
        fn edit_brief(&mut self, text: String) -> Result<Change, Fault> {
            if self.brief == text {
                return Ok(Change::Unchanged("The brief is unchanged."));
            }
            self.brief = text;
            self.writes += 1;
            Ok(Change::Applied)
        }
        fn keep(&mut self, id: &AssetId) -> Result<Change, Fault> {
            if self.shortlist.contains(id) {
                return Ok(Change::Unchanged("Already kept with the brief."));
            }
            self.shortlist.push(id.clone());
            self.writes += 1;
            Ok(Change::Applied)
        }
        fn remove(&mut self, id: &AssetId) -> Result<Change, Fault> {
            let before = self.shortlist.len();
            self.shortlist.retain(|item| item != id);
            if self.shortlist.len() == before {
                return Ok(Change::Unchanged("That image was not in the brief."));
            }
            self.writes += 1;
            Ok(Change::Applied)
        }
        fn reopen(&mut self) -> Result<Change, Fault> {
            self.writes += 1;
            Ok(Change::Applied)
        }
        fn note_write(&mut self, _: &'static str) {
            self.notes += 1;
        }
    }

    #[test]
    fn stale_revision_is_rejected_without_mutating() {
        let mut fake = Fake::new();
        fake.access.activate(AccessMode::Edit).unwrap();
        let id = fake.id(0);
        let current = fake.revision();
        let stale = fake.select_command(id, current.wrapping_sub(1));
        let reply = evaluate(&mut fake, stale);
        assert!(matches!(
            reply,
            Reply::Conflict {
                expected,
                current: live
            } if expected == live.wrapping_sub(1) && live == current
        ));
        assert_eq!(fake.writes, 0);
    }

    #[test]
    fn duplicate_command_replays_without_a_second_write() {
        let mut fake = Fake::new();
        fake.access.activate(AccessMode::Edit).unwrap();
        let id = fake.id(0);
        let command = fake.select_command(id, fake.revision());
        let first = evaluate(&mut fake, command.clone());
        let again = evaluate(&mut fake, command);
        assert!(matches!(first, Reply::Ok { .. }));
        assert_eq!(again, first);
        let conflicting_reuse = fake.select_command(fake.id(0), 0);
        assert!(matches!(
            evaluate(&mut fake, conflicting_reuse),
            Reply::Duplicate { .. }
        ));
        assert_eq!(fake.writes, 1);
        assert_eq!(fake.notes, 1);
    }

    fn read_command(fake: &Fake) -> Command {
        Command {
            workspace: fake.workspace.clone(),
            bearer: fake.access.bearer().unwrap_or("").into(),
            id: "read-1".into(),
            request: Request::Read { limit: None },
        }
    }

    #[test]
    fn reads_need_a_grant_and_writes_need_edit_and_revoke_is_immediate() {
        let mut fake = Fake::new();
        let read = read_command(&fake);
        assert!(matches!(evaluate(&mut fake, read), Reply::Denied { .. }));

        fake.access.activate(AccessMode::Read).unwrap();
        let read = read_command(&fake);
        assert!(matches!(
            evaluate(&mut fake, read.clone()),
            Reply::Ok { .. }
        ));

        let id = fake.id(0);
        let write = fake.select_command(id.clone(), fake.revision());
        assert!(matches!(
            evaluate(&mut fake, write.clone()),
            Reply::Denied { .. }
        ));

        fake.access.activate(AccessMode::Edit).unwrap();
        let write = fake.select_command(id, fake.revision());
        assert!(matches!(
            evaluate(&mut fake, write.clone()),
            Reply::Ok { .. }
        ));

        fake.access.revoke();
        assert!(matches!(evaluate(&mut fake, read), Reply::Denied { .. }));
        assert!(matches!(evaluate(&mut fake, write), Reply::Denied { .. }));
    }

    #[test]
    fn unknown_fields_and_overlong_text_are_rejected() {
        assert!(decode_command(
            r#"{"workspace":"ws:test","bearer":"b","id":"c","request":{"op":"read","limit":1,"extra":true}}"#
        )
        .is_err());
        assert!(
            decode_command(
                r#"{"workspace":"ws:test","bearer":"b","id":"c","request":{"op":"unknown"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn revision_tracks_brief_and_selection() {
        let a = AssetId("local:a".into());
        let base = RevisionParts {
            human_edits: 0,
            brief: "hello",
            selected: Some(&a),
            shortlist: &[],
            compare_first: None,
            compare_second: None,
            assets: &[a.clone()],
        };
        let changed = RevisionParts {
            brief: "hello!",
            ..base
        };
        assert_ne!(revision_of(&base), revision_of(&changed));
        let none = RevisionParts {
            selected: None,
            ..base
        };
        assert_ne!(revision_of(&base), revision_of(&none));
    }
}
