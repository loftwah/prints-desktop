//! Native connection and library screens. Owner media is held in memory for this session.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use prints_desktop::oyster::{
    ApiKey, Category, Cursor as PageCursor, Diagnostic, Error, Fallback, FolderPage, Model,
    OutputItem, OutputPage, OutputRead, OysterClient, ReadFailure, TransferCapture,
};
use rune::Latest;

use super::*;

const KEY_SERVICE: &str = "com.loftwah.prints-desktop.oyster";
const PROFILE: &str = "oyster-profile";
const PREVIEW_BYTES: u64 = 8 * 1024 * 1024;
const EXPORT_BYTES: u64 = 32 * 1024 * 1024;
/// A report carries a short trail, not an event log.
const TRAIL_LIMIT: usize = 10;
type Profile = (String, String);

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct Connection {
    client: Arc<OysterClient>,
    page: OutputRead,
    stored: Result<bool, String>,
    fallback: Option<Fallback>,
}

#[derive(Clone)]
struct PagePosition {
    folder: Option<String>,
    cursors: Vec<Option<PageCursor>>,
    index: usize,
}

impl PagePosition {
    fn first(folder: Option<String>) -> Self {
        Self {
            folder,
            cursors: vec![None],
            index: 0,
        }
    }

    fn previous(&self) -> Option<Self> {
        Some(Self {
            index: self.index.checked_sub(1)?,
            ..self.clone()
        })
    }

    fn next(&self, cursor: PageCursor) -> Self {
        let mut cursors = self.cursors[..=self.index].to_vec();
        cursors.push(Some(cursor));
        Self {
            folder: self.folder.clone(),
            cursors,
            index: self.index + 1,
        }
    }
}

/// Optional discovery owns its own state. It never occupies the library's
/// general busy flag, so selection, paging, preview and export stay available.
enum Catalogue {
    Pending { retry: u32 },
    Ready(Vec<Model>),
    Failed(Failure),
}

struct Library {
    client: Arc<OysterClient>,
    catalogue: Catalogue,
    position: PagePosition,
    folders: Option<FolderPage>,
    page: OutputPage,
    selected: Option<usize>,
    preview: Option<Image>,
    last_export: Option<PathBuf>,
}

impl Library {
    fn new(client: Arc<OysterClient>, read: OutputRead) -> Self {
        let selected = (!read.page.items.is_empty()).then_some(0);
        Self {
            client,
            catalogue: Catalogue::Pending { retry: 0 },
            position: PagePosition::first(None),
            folders: None,
            page: read.page,
            selected,
            preview: None,
            last_export: None,
        }
    }

    fn show(&mut self, read: OutputRead, position: PagePosition) {
        self.folders = None;
        self.selected = (!read.page.items.is_empty()).then_some(0);
        self.preview = None;
        self.page = read.page;
        self.position = position;
    }

    fn selected(&self) -> Option<&OutputItem> {
        self.page.items.get(self.selected?)
    }
}

#[derive(Clone, Copy)]
enum Transfer {
    Preview,
    Original,
}

enum Operation {
    Connect,
    Forget,
    LoadPage(PagePosition),
    BrowseFolders,
    Transfer(Transfer),
}

impl Operation {
    fn label(&self) -> &'static str {
        match self {
            Self::Connect => "Connecting to Oyster…",
            Self::Forget => "Removing saved key…",
            Self::LoadPage(_) => "Loading library…",
            Self::BrowseFolders => "Loading folders…",
            Self::Transfer(Transfer::Preview) => "Loading image preview…",
            Self::Transfer(Transfer::Original) => "Fetching original…",
        }
    }

    /// The typed call behind this operation, used as the report's method.
    fn method(&self) -> &'static str {
        match self {
            Self::Connect | Self::LoadPage(_) => "listOutputs",
            Self::BrowseFolders => "listFolders",
            Self::Transfer(_) => "openOutput",
            Self::Forget => "local",
        }
    }

    fn report_operation(&self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Forget => "forget saved key",
            Self::LoadPage(_) => "load library page",
            Self::BrowseFolders => "browse folders",
            Self::Transfer(Transfer::Preview) => "load preview",
            Self::Transfer(Transfer::Original) => "fetch original",
        }
    }
}

enum Reply {
    Connected(Connection),
    Page(OutputRead),
    Folders(FolderPage),
    Preview(Vec<u8>),
    Forgotten,
}

#[derive(Clone)]
struct Failure {
    message: String,
    /// A specific next action, or an honest statement that none is supported.
    recovery: &'static str,
    unauthorized: bool,
    diagnostic: Diagnostic,
}

impl Failure {
    fn from_error(
        operation: &'static str,
        method: &'static str,
        error: Error,
        elapsed: u64,
        fallback: Option<Fallback>,
    ) -> Self {
        let unauthorized = error.authority_lost();
        let diagnostic = error.diagnostic(operation, method, elapsed, fallback);
        let (message, recovery) = match &error {
            _ if unauthorized => (
                "The key was refused or revoked. Check your scoped App key.",
                "Reconnect with a current scoped App key.",
            ),
            Error::Refused { code, .. }
            | Error::Status {
                code: Some(code), ..
            } if code == "library_unavailable" => (
                "Choose a gallery folder in web Prints on the selected Oyster Computer, then reconnect.",
                "Set the library folder in web Prints, then reconnect.",
            ),
            Error::Status { status: 403, .. } => (
                "Oyster refused this action. Check the installation grants and selected Computer.",
                "Grant the required permission, then retry or reconnect.",
            ),
            Error::Status { status: 404, .. } => (
                "The installation or operation is unavailable on this host.",
                "Check the installation and host, then reconnect.",
            ),
            Error::Status {
                status: 502 | 503 | 504,
                ..
            }
            | Error::Transport => (
                "Oyster is temporarily unavailable. The library you already have is unchanged.",
                "Retry the operation. The library stays available.",
            ),
            Error::TooLarge => (
                "This file exceeds the transfer size or request limit.",
                "Open the original in web Prints instead.",
            ),
            Error::Decode => (
                "The reply was malformed, so no untrusted text was shown.",
                "Retry. The library you already have is unchanged.",
            ),
            Error::Refused { code, .. } if code == "not_declared" || code == "invalid_input" => (
                "This host does not support that operation.",
                "Use the root library or web Prints; no retry will change the host contract.",
            ),
            Error::Refused { code, .. }
                if code == "not_granted" || code == "requires_not_granted" =>
            {
                (
                    "This installation has not granted that capability.",
                    "Grant access in Oyster, then retry. The library stays available.",
                )
            }
            _ => (
                "The operation did not complete.",
                "Retry, or copy a connection report for the existing bug form.",
            ),
        };
        Self {
            message: message.into(),
            recovery,
            unauthorized,
            diagnostic,
        }
    }

    /// Catalogue failures get a capability-specific next step without ever
    /// disconnecting the readable library for a non-authority failure.
    fn catalogue(error: Error, elapsed: u64) -> Self {
        let mut failure = Self::from_error("discover models", "listModels", error, elapsed, None);
        // Confirmed shared-authority loss clears the library; never promise
        // that it is still available in that case.
        if failure.unauthorized {
            failure.message =
                "The key was refused or revoked. Reconnect with a current scoped App key.".into();
            failure.recovery = "Reconnect with a current scoped App key.";
            return failure;
        }
        failure.message = match failure.diagnostic.category {
            Category::Decode => "The model catalogue reply was malformed. Your library is still available.".into(),
            Category::Status | Category::Refused if failure.diagnostic.code.as_deref() == Some("not_granted") => {
                "Model discovery needs media.generation on this installation. Your library is still available.".into()
            }
            Category::Status | Category::Refused => {
                "Model discovery is unavailable for this connection. Your library is still available.".into()
            }
            Category::Transport => {
                "Model discovery could not reach Oyster. Your library is still available.".into()
            }
            _ => "Model discovery did not finish. Your library is still available.".into(),
        };
        failure
    }

    fn local(operation: &'static str, method: &'static str, message: &str) -> Self {
        Self {
            message: message.into(),
            recovery: "Try again, or copy a connection report for the bug form.",
            unauthorized: false,
            diagnostic: Diagnostic {
                operation,
                method,
                category: Category::Local,
                status: None,
                code: None,
                elapsed_ms: 0,
                fallback: None,
            },
        }
    }

    /// One short line for the report: status or category, then allowed code.
    fn cause(&self) -> String {
        let mut cause = match self.diagnostic.status {
            Some(status) => format!("http {status}"),
            None => self.diagnostic.category.label().to_owned(),
        };
        if let Some(code) = &self.diagnostic.code {
            cause.push_str(&format!(" code={code}"));
        }
        cause
    }
}

/// Which independent lane an observed event belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Library,
    Catalogue,
    Transfer,
}

impl Lane {
    fn label(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Catalogue => "catalogue",
            Self::Transfer => "transfer",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrailKind {
    Started,
    Completed,
    Failed,
    Superseded,
    Invalidated,
    Ignored,
}

impl TrailKind {
    fn label(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Superseded => "superseded",
            Self::Invalidated => "session invalidated",
            Self::Ignored => "obsolete result ignored",
        }
    }
}

/// A request's locally generated identity. It is never a key, host, Computer,
/// installation or file path, and it is only shown inside a local report.
#[derive(Clone)]
struct Pending {
    correlation: String,
    session: u64,
    started: Instant,
    method: &'static str,
}

struct TrailEvent {
    correlation: String,
    lane: Lane,
    method: &'static str,
    kind: TrailKind,
    elapsed_ms: u64,
    accepted: bool,
    detail: String,
}

/// A reply carries its own lane identity, so a result from an old operation or
/// session can be recognised and ignored rather than applied by coincidence.
struct Envelope {
    pending: Pending,
    diagnostic: Diagnostic,
    result: Result<Reply, Failure>,
}

struct CatalogueEnvelope {
    pending: Pending,
    result: Result<Vec<Model>, Failure>,
}

/// The identity of one original transfer. It carries the connection generation
/// and the seal over the immutable output capture, so an answer from another
/// selection, epoch or session can never be applied by coincidence.
#[derive(Clone)]
struct TransferPending {
    correlation: String,
    session: u64,
    epoch: u64,
    seal: u64,
    started: Instant,
}

enum TransferReply {
    Original(Vec<u8>),
    /// A cooperative stop was observed at a chunk boundary. No file was saved.
    Stopped,
}

struct TransferEnvelope {
    pending: TransferPending,
    diagnostic: Diagnostic,
    result: Result<TransferReply, Failure>,
}

/// One observed chunk, coalesced into a single slot and delivered through the
/// same Rune wake mechanism as the final answer. Cumulative byte counts mean
/// nothing is lost when intermediate observations are replaced.
struct TransferProgressEvent {
    pending: TransferPending,
    observed: u64,
}

/// A single coalesced progress slot. Publishing writes the newest observation
/// and calls the waker directly: no extra background worker is started per
/// chunk, and a superseded observation never lands after a newer one.
#[derive(Clone)]
struct ProgressSlot {
    inner: Arc<ProgressSlotInner>,
}

struct ProgressSlotInner {
    value: Mutex<Option<TransferProgressEvent>>,
    wake: Mutex<Option<Waker>>,
}

impl ProgressSlot {
    fn new() -> Self {
        Self {
            inner: Arc::new(ProgressSlotInner {
                value: Mutex::new(None),
                wake: Mutex::new(None),
            }),
        }
    }

    fn set_waker(&self, wake: Waker) {
        *self.inner.wake.lock().unwrap() = Some(wake);
    }

    fn publish(&self, event: TransferProgressEvent) {
        *self.inner.value.lock().unwrap() = Some(event);
        if let Some(wake) = self.inner.wake.lock().unwrap().clone() {
            wake();
        }
    }

    fn take(&self) -> Option<TransferProgressEvent> {
        self.inner.value.lock().unwrap().take()
    }

    fn clear(&self) {
        *self.inner.value.lock().unwrap() = None;
    }
}

/// Original transfers own their own state. They never occupy the library's
/// `operation`, so browsing, selection and preview stay live during a fetch.
enum TransferState {
    Idle,
    Fetching {
        capture: TransferCapture,
        pending: TransferPending,
        observed: u64,
        total: Option<u64>,
        cancel: Arc<AtomicBool>,
    },
    /// A stop was requested. The blocking call already in flight is allowed to
    /// finish, but its bytes are dropped and no dialog may follow.
    Stopping {
        pending: TransferPending,
    },
    Dialog {
        capture: TransferCapture,
        pending: TransferPending,
        /// Exact owner of the open Save dialog. Its correlation is local to
        /// the dialog, not to the fetch that preceded it.
        dialog: Pending,
        bytes: Vec<u8>,
    },
    /// The user accepted a destination. The local write finishes atomically
    /// here, still on the transfer lane, without occupying library `operation`.
    Writing {
        pending: Pending,
        write: OriginalWrite,
    },
    Failed {
        capture: TransferCapture,
    },
}

pub(super) struct Hosted {
    origin: TextEdit,
    installation: TextEdit,
    key: TextEdit,
    remember: bool,
    saved: Option<Profile>,
    library: Option<Library>,
    folder: TextEdit,
    operation: Option<Operation>,
    status: String,
    /// Export callbacks and both request lanes are bound to this generation.
    epoch: u64,
    session: u64,
    next_id: u64,
    request: Latest<Envelope>,
    catalogue_request: Latest<CatalogueEnvelope>,
    /// Third independent lane; progress is one coalesced slot, not a task per
    /// chunk. Both reach the UI through the same waker as the other lanes.
    transfer_request: Latest<TransferEnvelope>,
    transfer_progress: ProgressSlot,
    transfer: TransferState,
    primary_pending: Option<Pending>,
    catalogue_pending: Option<Pending>,
    catalogue_retry: u32,
    trail: VecDeque<TrailEvent>,
    last_failure: Option<Failure>,
    last_catalogue_failure: Option<Failure>,
    last_fallback: Option<Fallback>,
    report_expanded: bool,
}

fn failure_from_error(
    operation: &'static str,
    method: &'static str,
    started: Instant,
    error: Error,
    fallback: Option<Fallback>,
) -> Failure {
    Failure::from_error(operation, method, error, elapsed_ms(started), fallback)
}

fn failure_from_read(
    operation: &'static str,
    method: &'static str,
    started: Instant,
    failure: ReadFailure,
) -> Failure {
    Failure::from_error(
        operation,
        method,
        failure.error,
        elapsed_ms(started),
        failure.fallback,
    )
}

impl Hosted {
    pub(super) fn new(prefs: &Prefs) -> Self {
        let saved: Option<Profile> = prefs.get(PROFILE);
        let (origin, installation) = saved
            .clone()
            .unwrap_or_else(|| (String::new(), "prints".into()));
        Self {
            origin: TextEdit::new(origin),
            installation: TextEdit::new(installation),
            key: TextEdit::new("").secure(),
            remember: false,
            saved,
            library: None,
            folder: TextEdit::new(""),
            operation: None,
            status: String::new(),
            epoch: 0,
            session: 0,
            next_id: 0,
            request: Latest::default(),
            catalogue_request: Latest::default(),
            transfer_request: Latest::default(),
            transfer_progress: ProgressSlot::new(),
            transfer: TransferState::Idle,
            primary_pending: None,
            catalogue_pending: None,
            catalogue_retry: 0,
            trail: VecDeque::new(),
            last_failure: None,
            last_catalogue_failure: None,
            last_fallback: None,
            report_expanded: false,
        }
    }

    pub(super) fn set_waker(&mut self, wake: Waker) {
        self.request.set_waker(wake.clone());
        self.catalogue_request.set_waker(wake.clone());
        self.transfer_request.set_waker(wake.clone());
        self.transfer_progress.set_waker(wake);
    }

    pub(super) fn connected(&self) -> bool {
        self.library.is_some()
    }

    fn busy(&self) -> bool {
        self.operation.is_some()
    }

    /// One hosted transfer at a time. Fetching, stopping, an open Save dialog
    /// and an accepted write all block a second transfer, so two dialogs can
    /// never be queued.
    fn transfer_busy(&self) -> bool {
        !matches!(
            &self.transfer,
            TransferState::Idle | TransferState::Failed { .. }
        )
    }

    fn transfer_pending(&self) -> Option<&TransferPending> {
        match &self.transfer {
            TransferState::Fetching { pending, .. }
            | TransferState::Stopping { pending, .. }
            | TransferState::Dialog { pending, .. } => Some(pending),
            TransferState::Writing { .. } | TransferState::Idle | TransferState::Failed { .. } => {
                None
            }
        }
    }

    fn log_transfer_event(
        &mut self,
        pending: &TransferPending,
        kind: TrailKind,
        elapsed_ms: u64,
        accepted: bool,
        detail: &str,
    ) {
        if self.trail.len() >= TRAIL_LIMIT {
            self.trail.pop_front();
        }
        self.trail.push_back(TrailEvent {
            correlation: pending.correlation.clone(),
            lane: Lane::Transfer,
            method: "openOutput",
            kind,
            elapsed_ms,
            accepted,
            detail: detail.chars().take(90).collect(),
        });
    }

    fn next_correlation(&mut self) -> String {
        self.next_id = self.next_id.wrapping_add(1);
        format!("op-{}", self.next_id)
    }

    fn log_event(
        &mut self,
        pending: &Pending,
        lane: Lane,
        kind: TrailKind,
        elapsed_ms: u64,
        accepted: bool,
        detail: &str,
    ) {
        if self.trail.len() >= TRAIL_LIMIT {
            self.trail.pop_front();
        }
        self.trail.push_back(TrailEvent {
            correlation: pending.correlation.clone(),
            lane,
            method: pending.method,
            kind,
            elapsed_ms,
            accepted,
            detail: detail.chars().take(90).collect(),
        });
    }

    fn start(&mut self, operation: Operation) -> Pending {
        if let Some(previous) = self.primary_pending.take() {
            self.log_event(
                &previous,
                Lane::Library,
                TrailKind::Superseded,
                elapsed_ms(previous.started),
                false,
                "new library request started; the older answer will be ignored",
            );
        }
        let pending = Pending {
            correlation: self.next_correlation(),
            session: self.session,
            started: Instant::now(),
            method: operation.method(),
        };
        self.primary_pending = Some(pending.clone());
        self.log_event(&pending, Lane::Library, TrailKind::Started, 0, false, "");
        self.operation = Some(operation);
        self.status.clear();
        pending
    }

    /// Start model discovery on its own request. Nothing here takes the
    /// library's busy flag or changes the selected item, page or preview.
    fn start_catalogue(&mut self) {
        let Some(library) = &self.library else {
            return;
        };
        let client = library.client.clone();
        let retry = match &library.catalogue {
            Catalogue::Pending { retry } => *retry,
            Catalogue::Failed(_) => self.catalogue_retry,
            Catalogue::Ready(_) => 0,
        };
        if let Some(previous) = self.catalogue_pending.take() {
            self.log_event(
                &previous,
                Lane::Catalogue,
                TrailKind::Superseded,
                elapsed_ms(previous.started),
                false,
                "new discovery started; the older answer will be ignored",
            );
        }
        let pending = Pending {
            correlation: self.next_correlation(),
            session: self.session,
            started: Instant::now(),
            method: "listModels",
        };
        self.catalogue_pending = Some(pending.clone());
        self.log_event(&pending, Lane::Catalogue, TrailKind::Started, 0, false, "");
        if let Some(library) = &mut self.library {
            library.catalogue = Catalogue::Pending { retry };
        }
        self.catalogue_request
            .request((client, pending.clone()), move |(client, pending)| {
                let started = Instant::now();
                let result = client
                    .list_models()
                    .map(|catalogue| catalogue.models)
                    .map_err(|error| Failure::catalogue(error, elapsed_ms(started)));
                CatalogueEnvelope { pending, result }
            });
    }

    fn retry_catalogue(&mut self) {
        if !self.connected()
            || self
                .library
                .as_ref()
                .is_some_and(|library| matches!(library.catalogue, Catalogue::Pending { .. }))
        {
            return;
        }
        self.catalogue_retry = self.catalogue_retry.saturating_add(1);
        self.start_catalogue();
    }

    fn connect(&mut self, saved: bool) {
        if self.busy() || self.connected() {
            return;
        }
        let profile = if saved {
            let Some(profile) = self.saved.clone() else {
                return;
            };
            profile
        } else {
            (
                self.origin.text.trim().into(),
                self.installation.text.trim().into(),
            )
        };
        let key = self.key.text.trim().to_owned();
        let remember = self.remember && !saved;
        let saved_profile = self.saved.clone();
        self.key = TextEdit::new("").secure();
        // A fresh attempt begins a new local session. Only its replies may land.
        self.session = self.session.wrapping_add(1);
        let pending = self.start(Operation::Connect);
        self.request
            .request((profile, key, pending.clone()), move |(profile, entered, pending)| {
                let started = Instant::now();
                let run = || -> Result<Connection, Failure> {
                    let (origin, installation) = profile;
                    let key = if saved {
                        credential(&(origin.clone(), installation.clone()))?
                            .get_password()
                            .map_err(|_| {
                                Failure::local(
                                    "connect",
                                    "credential",
                                    "The saved key is unavailable. Enter a current key to connect.",
                                )
                            })?
                    } else {
                        entered
                    };
                    let client = Arc::new(
                        OysterClient::new(&origin, &installation, ApiKey::new(key.clone()))
                            .map_err(|error| {
                                failure_from_error("connect", "listOutputs", started, error, None)
                            })?,
                    );
                    // The authorised read alone opens the library. Model
                    // discovery starts only after this reply is accepted.
                    let read = client
                        .list_outputs_detailed(None, true, None, 24)
                        .map_err(|failure| {
                            failure_from_read("connect", "listOutputs", started, failure)
                        })?;
                    let fallback = read.fallback;
                    let stored = if remember
                        && saved_profile.as_ref().is_some_and(|old| old != &client.identity())
                    {
                        Err("Connected for this session. Forget the previous saved key before saving a different connection.".into())
                    } else if remember {
                        credential(&client.identity())
                            .and_then(|entry| {
                                entry.set_password(&key).map_err(|_| {
                                    Failure::local(
                                        "connect",
                                        "credential",
                                        "The OS credential store couldn't save this key.",
                                    )
                                })
                            })
                            .map(|_| true)
                            .map_err(|error| error.message)
                    } else {
                        Ok(false)
                    };
                    Ok(Connection {
                        client,
                        page: read,
                        stored,
                        fallback,
                    })
                };
                let result = run().map(Reply::Connected);
                let elapsed = elapsed_ms(started);
                let diagnostic = match &result {
                    Ok(Reply::Connected(connection)) => Diagnostic::success(
                        "connect",
                        "listOutputs",
                        elapsed,
                        connection.fallback,
                    ),
                    Err(failure) => failure.diagnostic.clone(),
                    _ => unreachable!(),
                };
                Envelope {
                    pending,
                    diagnostic,
                    result,
                }
            });
    }

    fn disconnect(&mut self) {
        self.request.cancel();
        self.catalogue_request.cancel();
        self.transfer_request.cancel();
        self.transfer_progress.clear();
        if let TransferState::Fetching { cancel, .. } = &self.transfer {
            // Stop future chunk dispatch too; the channel cancel alone only
            // drops a late answer, it does not reach a running worker.
            cancel.store(true, Ordering::Release);
        }
        if let Some(pending) = self.transfer_pending().cloned() {
            self.log_transfer_event(
                &pending,
                TrailKind::Invalidated,
                elapsed_ms(pending.started),
                false,
                "connection ended; a late answer is ignored, transport not known to be cancelled",
            );
        }
        self.transfer = TransferState::Idle;
        if let Some(pending) = self.primary_pending.take() {
            self.log_event(
                &pending,
                Lane::Library,
                TrailKind::Invalidated,
                elapsed_ms(pending.started),
                false,
                "connection ended; a late answer is ignored, transport not known to be cancelled",
            );
        }
        if let Some(pending) = self.catalogue_pending.take() {
            self.log_event(
                &pending,
                Lane::Catalogue,
                TrailKind::Invalidated,
                elapsed_ms(pending.started),
                false,
                "connection ended; a late answer is ignored, transport not known to be cancelled",
            );
        }
        self.library = None;
        self.folder = TextEdit::new("");
        self.key = TextEdit::new("").secure();
        self.operation = None;
        self.status.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.session = self.session.wrapping_add(1);
    }

    pub(super) fn finish_write(&mut self) {
        if matches!(
            &self.transfer,
            TransferState::Dialog { .. } | TransferState::Writing { .. }
        ) {
            self.transfer = TransferState::Idle;
        }
    }

    fn forget(&mut self) {
        if self.busy() || self.connected() {
            return;
        }
        let Some(profile) = self.saved.clone() else {
            return;
        };
        let pending = self.start(Operation::Forget);
        self.request
            .request((profile, pending.clone()), move |(profile, pending)| {
                let started = Instant::now();
                let result = (|| -> Result<Reply, Failure> {
                    match credential(&profile)?.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => Ok(Reply::Forgotten),
                        Err(_) => Err(Failure::local(
                            "forget saved key",
                            "credential",
                            "The OS credential store couldn't remove the key.",
                        )),
                    }
                })();
                let diagnostic = match &result {
                    Ok(_) => Diagnostic::success(
                        "forget saved key",
                        "credential",
                        elapsed_ms(started),
                        None,
                    ),
                    Err(failure) => failure.diagnostic.clone(),
                };
                Envelope {
                    pending,
                    diagnostic,
                    result,
                }
            });
    }

    fn load_page(&mut self, position: PagePosition) {
        if self.busy() {
            return;
        }
        let Some(library) = &self.library else {
            return;
        };
        let client = library.client.clone();
        let cursor = position.cursors[position.index].clone();
        let folder = position.folder.clone();
        let pending = self.start(Operation::LoadPage(position));
        self.request.request(
            (client, folder, cursor, pending.clone()),
            move |(client, folder, cursor, pending)| {
                let started = Instant::now();
                let result = client
                    .list_outputs_detailed(folder.as_deref(), true, cursor, 24)
                    .map(Reply::Page)
                    .map_err(|failure| {
                        failure_from_read("load library page", "listOutputs", started, failure)
                    });
                let diagnostic = match &result {
                    Ok(Reply::Page(read)) => Diagnostic::success(
                        "load library page",
                        "listOutputs",
                        elapsed_ms(started),
                        read.fallback,
                    ),
                    Err(failure) => failure.diagnostic.clone(),
                    _ => unreachable!(),
                };
                Envelope {
                    pending,
                    diagnostic,
                    result,
                }
            },
        );
    }

    fn browse_folders(&mut self, after: Option<u64>) {
        if self.busy() {
            return;
        }
        let Some(library) = &self.library else {
            return;
        };
        let client = library.client.clone();
        let folder = library.position.folder.clone();
        let pending = self.start(Operation::BrowseFolders);
        self.request.request(
            (client, folder, pending.clone()),
            move |(client, folder, pending)| {
                let started = Instant::now();
                let result = client
                    .library_folders(folder.as_deref(), after)
                    .map(Reply::Folders)
                    .map_err(|error| {
                        failure_from_error("browse folders", "listFolders", started, error, None)
                    });
                let diagnostic = match &result {
                    Ok(_) => Diagnostic::success(
                        "browse folders",
                        "listFolders",
                        elapsed_ms(started),
                        None,
                    ),
                    Err(failure) => failure.diagnostic.clone(),
                };
                Envelope {
                    pending,
                    diagnostic,
                    result,
                }
            },
        );
    }

    fn apply_folder(&mut self) {
        let folder = self.folder.text.trim();
        let folder = (!folder.is_empty()).then(|| folder.to_owned());
        self.load_page(PagePosition::first(folder));
    }

    fn select(&mut self, index: usize) {
        if !self.busy()
            && let Some(library) = &mut self.library
            && index < library.page.items.len()
        {
            library.selected = Some(index);
            library.preview = None;
            self.status.clear();
        }
    }

    fn transfer(&mut self, transfer: Transfer) {
        match transfer {
            Transfer::Preview => self.load_preview(),
            Transfer::Original => self.start_original_transfer(None),
        }
    }

    /// A preview is short and bounded, so it keeps the library lane. It stays
    /// available while an original transfer runs on its own lane.
    fn load_preview(&mut self) {
        if self.busy() {
            return;
        }
        let Some(library) = &self.library else {
            return;
        };
        let Some(item) = library.selected().cloned() else {
            return;
        };
        if item.print.bytes > PREVIEW_BYTES {
            self.status = format!(
                "This original exceeds the {} MiB limit.",
                PREVIEW_BYTES / (1024 * 1024)
            );
            return;
        }
        let client = library.client.clone();
        let operation = Operation::Transfer(Transfer::Preview);
        let (report_operation, method) = (operation.report_operation(), operation.method());
        let pending = self.start(operation);
        self.request.request(
            (client, item, pending.clone()),
            move |(client, item, pending)| {
                let started = Instant::now();
                let result = client
                    .verify_and_download(&item)
                    .map_err(|error| {
                        failure_from_error(report_operation, method, started, error, None)
                    })
                    .and_then(|bytes| {
                        if previewable(&bytes) {
                            Ok(Reply::Preview(bytes))
                        } else {
                            Err(Failure::local(
                                report_operation,
                                method,
                                "This image format can be exported or opened in web Prints.",
                            ))
                        }
                    });
                let diagnostic = match &result {
                    Ok(_) => {
                        Diagnostic::success(report_operation, method, elapsed_ms(started), None)
                    }
                    Err(failure) => failure.diagnostic.clone(),
                };
                Envelope {
                    pending,
                    diagnostic,
                    result,
                }
            },
        );
    }

    /// Start one original transfer bound to an immutable capture. A retry
    /// reuses the capture from the failed attempt, never the current selection.
    fn start_original_transfer(&mut self, reuse: Option<TransferCapture>) {
        if self.transfer_busy() {
            return;
        }
        let correlation = self.next_correlation();
        let Some(library) = &self.library else {
            return;
        };
        let client = library.client.clone();
        let epoch = self.epoch;
        let session = self.session;
        // A retry rebinds the captured item to the current generation and the
        // new correlation. The seal must match the correlation actually sent;
        // the user's current selection is never consulted here.
        let capture = match reuse {
            Some(previous) => TransferCapture::new(previous.item(), epoch, &correlation),
            None => {
                let Some(item) = library.selected() else {
                    return;
                };
                if item.print.bytes > EXPORT_BYTES {
                    self.status = format!(
                        "This original exceeds the {} MiB limit.",
                        EXPORT_BYTES / (1024 * 1024)
                    );
                    return;
                }
                TransferCapture::new(item, epoch, &correlation)
            }
        };
        let pending = TransferPending {
            correlation,
            session,
            epoch,
            seal: capture.seal(),
            started: Instant::now(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = self.transfer_progress.clone();
        let total = capture.total();
        self.transfer = TransferState::Fetching {
            capture: capture.clone(),
            pending: pending.clone(),
            observed: 0,
            total,
            cancel: cancel.clone(),
        };
        self.status.clear();
        self.log_transfer_event(&pending, TrailKind::Started, 0, false, "");
        self.transfer_request.request(
            (client, capture, pending, cancel, progress),
            move |(client, capture, pending, cancel, progress)| {
                let started = Instant::now();
                let result = client.transfer_original(&capture, &cancel, |update| {
                    if !cancel.load(Ordering::Acquire) {
                        progress.publish(TransferProgressEvent {
                            pending: pending.clone(),
                            observed: update.observed,
                        });
                    }
                });
                let result = match result {
                    Ok(bytes) => Ok(TransferReply::Original(bytes)),
                    Err(Error::Cancelled) => Ok(TransferReply::Stopped),
                    Err(error) => Err(failure_from_error(
                        "fetch original",
                        "openOutput",
                        started,
                        error,
                        None,
                    )),
                };
                let diagnostic = match &result {
                    Ok(_) => Diagnostic::success(
                        "fetch original",
                        "openOutput",
                        elapsed_ms(started),
                        None,
                    ),
                    Err(failure) => failure.diagnostic.clone(),
                };
                TransferEnvelope {
                    pending,
                    diagnostic,
                    result,
                }
            },
        );
    }

    /// Cooperative stop. The flag is checked before each next chunk; a request
    /// already in flight is not claimed to be aborted, and its bytes are
    /// dropped before any dialog can be opened.
    fn cancel_transfer(&mut self) {
        let stop = match &self.transfer {
            TransferState::Fetching {
                pending, cancel, ..
            } if !cancel.load(Ordering::Acquire) => Some((pending.clone(), cancel.clone())),
            _ => None,
        };
        if let Some((pending, cancel)) = stop {
            cancel.store(true, Ordering::Release);
            self.log_transfer_event(
                &pending,
                TrailKind::Invalidated,
                elapsed_ms(pending.started),
                false,
                "stop requested; in-flight request not known to be cancelled",
            );
            self.transfer = TransferState::Stopping { pending };
            self.status = "Stopping the transfer. No file will be saved.".into();
        }
    }

    fn retry_transfer(&mut self) {
        let capture = match &self.transfer {
            TransferState::Failed { capture, .. } => Some(capture.clone()),
            _ => None,
        };
        if let Some(capture) = capture {
            self.transfer = TransferState::Idle;
            self.start_original_transfer(Some(capture));
        }
    }

    /// Short, actionable next step drawn only from typed evidence.
    fn summary(&self) -> String {
        if let Some(failure) = &self.last_catalogue_failure {
            return format!("{} {}", failure.message, failure.recovery);
        }
        if let Some(failure) = &self.last_failure {
            return format!("{} {}", failure.message, failure.recovery);
        }
        if let Some(library) = &self.library
            && matches!(library.catalogue, Catalogue::Pending { .. })
        {
            return "Discovering models is optional. Browsing, preview and export stay available."
                .into();
        }
        "No connection problem recorded. Review the report before pasting it into the bug form."
            .into()
    }

    /// A deliberately local report. Every field is typed or fixed; no key,
    /// header, body, path, host, Computer, installation, artwork or prompt is
    /// read from the request or the loaded library.
    fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("Prints Desktop connection report\n");
        out.push_str(&format!("App: {}\n", env!("CARGO_PKG_VERSION")));
        out.push_str(&format!(
            "Source revision: {}\n",
            option_env!("PRINTS_SOURCE_REVISION")
                .filter(|revision| revision.len() == 40
                    && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .unwrap_or("unknown")
        ));
        out.push_str(&format!(
            "Platform: {} {}\n",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        out.push_str(&format!(
            "Library: {}\n",
            if self.connected() {
                "connected in this session"
            } else {
                "not connected"
            }
        ));
        if let Some(fallback) = self.last_fallback {
            out.push_str(&format!("Compatibility fallback: {}\n", fallback.note()));
        }
        let mut recorded = false;
        for (label, failure) in [
            ("Library failure", self.last_failure.as_ref()),
            ("Catalogue failure", self.last_catalogue_failure.as_ref()),
        ] {
            if let Some(failure) = failure {
                recorded = true;
                let diagnostic = &failure.diagnostic;
                let fallback = diagnostic
                    .fallback
                    .map(Fallback::note)
                    .unwrap_or("none observed");
                out.push_str(&format!(
                    "{label}: {} {} ({}) {}ms fallback={} code={}\n",
                    diagnostic.operation,
                    diagnostic.method,
                    failure.cause(),
                    diagnostic.elapsed_ms,
                    fallback,
                    diagnostic.code.as_deref().unwrap_or("none"),
                ));
                out.push_str(&format!("Next step: {}\n", failure.recovery));
            }
        }
        if !recorded {
            out.push_str("Failures: none recorded in this session\n");
        }
        out.push_str("Trail:\n");
        for event in &self.trail {
            out.push_str(&format!(
                "- {} {} {} {} {}ms accepted={} {}\n",
                event.correlation,
                event.lane.label(),
                event.method,
                event.kind.label(),
                event.elapsed_ms,
                event.accepted,
                event.detail,
            ));
        }
        out
    }
}

fn credential(profile: &Profile) -> Result<keyring::Entry, Failure> {
    if crate::tooling_mode::isolated() {
        return Err(Failure::local(
            "connect",
            "credential",
            "Verification profiles use session-only keys.",
        ));
    }
    let (origin, installation) = profile;
    keyring::Entry::new(KEY_SERVICE, &format!("{origin}/{installation}")).map_err(|_| {
        Failure::local(
            "credential",
            "credential",
            "Your OS credential store is unavailable.",
        )
    })
}

fn previewable(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

fn filename(path: &str) -> String {
    let raw = path.rsplit('/').next().unwrap_or("print");
    let safe: String = raw
        .chars()
        .take(120)
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = safe.trim().trim_matches('.');
    if safe.is_empty() {
        "print".into()
    } else {
        safe.into()
    }
}

pub(super) fn poll(s: &mut Studio, _: f64, tx: &mut Tx) -> bool {
    let mut updated = false;
    // Discovery is polled first and on its own: an export dialog, a pending
    // write or a library reply cannot hide its completion.
    let catalogue = s.hosted.catalogue_request.take();
    if let Some(envelope) = catalogue {
        updated = true;
        apply_catalogue(s, tx, envelope);
    }
    let library = s.hosted.request.take();
    if let Some(envelope) = library {
        updated = true;
        apply_library(s, tx, envelope);
    }
    // Observed chunks and the final answer arrive on the same waker but in
    // separate slots, so a late progress event cannot swallow a completion.
    if let Some(progress) = s.hosted.transfer_progress.take() {
        updated = true;
        apply_transfer_progress(s, progress);
    }
    if let Some(transfer) = s.hosted.transfer_request.take() {
        updated = true;
        apply_transfer(s, tx, transfer);
    }
    // An accepted local write completes atomically on the transfer lane. It
    // never borrows the library busy flag or invalidates its primary pending.
    let written = match &s.hosted.transfer {
        TransferState::Writing { write, .. } => write.take(),
        _ => None,
    };
    if let Some(result) = written {
        let pending = match std::mem::replace(&mut s.hosted.transfer, TransferState::Idle) {
            TransferState::Writing { pending, .. } => pending,
            _ => unreachable!(),
        };
        let (kind, detail) = match &result {
            Ok(_) => (TrailKind::Completed, "original written"),
            Err(_) => (TrailKind::Failed, "original write failed"),
        };
        let elapsed = elapsed_ms(pending.started);
        s.hosted
            .log_event(&pending, Lane::Library, kind, elapsed, true, detail);
        match result {
            Ok(path) => {
                if let Some(library) = &mut s.hosted.library {
                    library.last_export = Some(path);
                }
                s.hosted.status = "Original exported.".into();
            }
            Err(_) => s.hosted.status = "Couldn't write the selected file.".into(),
        }
        updated = true;
    }
    updated
}

/// Observed chunks only update the current fetch. A stale progress event, a
/// cancelled transfer or a changed connection is ignored without touching the
/// page, selection, preview or status.
fn apply_transfer_progress(s: &mut Studio, event: TransferProgressEvent) {
    let h = &mut s.hosted;
    let (epoch, session) = (h.epoch, h.session);
    if let TransferState::Fetching {
        pending, observed, ..
    } = &mut h.transfer
        && pending.correlation == event.pending.correlation
        && pending.epoch == epoch
        && pending.epoch == event.pending.epoch
        && pending.session == session
        && pending.session == event.pending.session
    {
        *observed = event.observed;
    }
}

/// Deliver one transfer-lane answer only when its generation, seal, session
/// and correlation still match. Obsolete answers are logged as ignored; no
/// Save dialog or library change may follow them.
fn apply_transfer(s: &mut Studio, tx: &mut Tx, envelope: TransferEnvelope) {
    let h = &mut s.hosted;
    if let TransferState::Stopping { pending, .. } = &h.transfer
        && pending.correlation == envelope.pending.correlation
    {
        h.log_transfer_event(
            &envelope.pending,
            TrailKind::Ignored,
            envelope.diagnostic.elapsed_ms,
            false,
            "stop requested; in-flight request not known to be cancelled",
        );
        h.transfer = TransferState::Idle;
        return;
    }
    let current = match &h.transfer {
        TransferState::Fetching {
            capture, pending, ..
        } => {
            pending.correlation == envelope.pending.correlation
                && pending.epoch == h.epoch
                && pending.epoch == envelope.pending.epoch
                && pending.session == h.session
                && pending.session == envelope.pending.session
                && pending.seal == envelope.pending.seal
                && capture.is_sealed_by(pending.epoch, &pending.correlation)
        }
        _ => false,
    };
    if !current {
        h.log_transfer_event(
            &envelope.pending,
            TrailKind::Ignored,
            envelope.diagnostic.elapsed_ms,
            false,
            "reply belonged to an earlier transfer or connection",
        );
        return;
    }
    let (capture, pending) = match std::mem::replace(&mut h.transfer, TransferState::Idle) {
        TransferState::Fetching {
            capture, pending, ..
        } => (capture, pending),
        _ => unreachable!(),
    };
    match envelope.result {
        Ok(TransferReply::Stopped) => {
            h.log_transfer_event(
                &pending,
                TrailKind::Invalidated,
                envelope.diagnostic.elapsed_ms,
                false,
                "stopped at a chunk boundary; no Save dialog",
            );
            h.status = "Transfer stopped. No file was saved.".into();
        }
        Ok(TransferReply::Original(bytes)) => {
            h.log_transfer_event(
                &pending,
                TrailKind::Completed,
                envelope.diagnostic.elapsed_ms,
                true,
                "",
            );
            // The dialog owns a locally generated identity of its own. Using
            // `start` here would supersede an in-flight page or preview and
            // would make the callback test against the wrong correlation.
            let dialog = Pending {
                correlation: h.next_correlation(),
                session: h.session,
                started: Instant::now(),
                method: "local",
            };
            h.log_event(
                &dialog,
                Lane::Library,
                TrailKind::Started,
                0,
                false,
                "save dialog opened",
            );
            h.transfer = TransferState::Dialog {
                capture: capture.clone(),
                pending: pending.clone(),
                dialog: dialog.clone(),
                bytes,
            };
            h.status = "Choose where to save the original…".into();
            let epoch = h.epoch;
            let session = h.session;
            let seal = capture.seal();
            let dialog_correlation = dialog.correlation.clone();
            tx.save_file(
                FileDialogOptions {
                    title: "Export Prints original".into(),
                    file_name: Some(filename(capture.path())),
                    ..FileDialogOptions::default()
                },
                move |s: &mut Studio, result, _| {
                    let h = &mut s.hosted;
                    if !dialog_is_current(h, epoch, session, seal, &dialog_correlation) {
                        return;
                    }
                    match result {
                        Ok(Some(path)) => {
                            let (dialog, bytes) =
                                match std::mem::replace(&mut h.transfer, TransferState::Idle) {
                                    TransferState::Dialog { dialog, bytes, .. } => (dialog, bytes),
                                    _ => return,
                                };
                            h.log_event(
                                &dialog,
                                Lane::Library,
                                TrailKind::Completed,
                                elapsed_ms(dialog.started),
                                true,
                                "destination accepted; write starting",
                            );
                            // One accepted destination starts exactly one local
                            // write, still owned by the transfer lane.
                            let write_pending = Pending {
                                correlation: h.next_correlation(),
                                session: h.session,
                                started: Instant::now(),
                                method: "local",
                            };
                            h.status = "Writing original…".into();
                            match OriginalWrite::start(path, bytes.into(), s.wake.clone()) {
                                Ok(write) => {
                                    h.log_event(
                                        &write_pending,
                                        Lane::Library,
                                        TrailKind::Started,
                                        0,
                                        false,
                                        "",
                                    );
                                    h.transfer = TransferState::Writing {
                                        pending: write_pending,
                                        write,
                                    };
                                }
                                Err(_) => {
                                    h.status = "Couldn't write the selected file.".into();
                                }
                            }
                        }
                        Ok(None) => {
                            let dialog =
                                match std::mem::replace(&mut h.transfer, TransferState::Idle) {
                                    TransferState::Dialog { dialog, .. } => dialog,
                                    _ => return,
                                };
                            h.log_event(
                                &dialog,
                                Lane::Library,
                                TrailKind::Completed,
                                elapsed_ms(dialog.started),
                                false,
                                "destination declined",
                            );
                            h.status = "Export cancelled.".into();
                        }
                        Err(_) => {
                            let dialog =
                                match std::mem::replace(&mut h.transfer, TransferState::Idle) {
                                    TransferState::Dialog { dialog, .. } => dialog,
                                    _ => return,
                                };
                            h.log_event(
                                &dialog,
                                Lane::Library,
                                TrailKind::Failed,
                                elapsed_ms(dialog.started),
                                false,
                                "save dialog failed",
                            );
                            h.status = "Couldn't open the Save dialog.".into();
                        }
                    }
                },
            );
        }
        Err(failure) => {
            h.log_transfer_event(
                &pending,
                TrailKind::Failed,
                envelope.diagnostic.elapsed_ms,
                true,
                &failure.cause(),
            );
            h.last_failure = Some(failure.clone());
            if failure.unauthorized {
                // Shared authority is gone. Disconnect clears the library,
                // both older lanes and every transfer capture.
                h.disconnect();
                if s.screen == Screen::Hosted {
                    s.screen = Screen::Connection;
                    tx.blur();
                }
            } else {
                // Retry stays bound to the captured output, not the selection
                // the user may have moved to while the fetch was running.
                h.transfer = TransferState::Failed { capture };
            }
            h.status = format!("{} {}", failure.message, failure.recovery);
        }
    }
}

/// Exact ownership of the open Save dialog. The dialog has its own correlation
/// because opening it must not supersede a page or preview reply in flight.
fn dialog_is_current(h: &Hosted, epoch: u64, session: u64, seal: u64, correlation: &str) -> bool {
    h.epoch == epoch
        && h.session == session
        && matches!(
            &h.transfer,
            TransferState::Dialog { capture, dialog, .. }
                if capture.seal() == seal && dialog.correlation == correlation
        )
}

/// Deliver one library-lane reply only when it belongs to the current
/// operation and local session. An obsolete reply is reported as ignored; the
/// transport is not claimed to have been cancelled.
fn apply_library(s: &mut Studio, tx: &mut Tx, envelope: Envelope) {
    let h = &mut s.hosted;
    let current = h.primary_pending.as_ref().is_some_and(|pending| {
        pending.correlation == envelope.pending.correlation && pending.session == h.session
    });
    if !current {
        h.log_event(
            &envelope.pending,
            Lane::Library,
            TrailKind::Ignored,
            envelope.diagnostic.elapsed_ms,
            false,
            "reply belonged to an earlier operation or session",
        );
        return;
    }
    let Some(pending) = h.primary_pending.take() else {
        return;
    };
    let Some(operation) = h.operation.take() else {
        return;
    };
    match (operation, envelope.result) {
        (Operation::Connect, Ok(Reply::Connected(connection))) => {
            h.log_event(
                &pending,
                Lane::Library,
                TrailKind::Completed,
                envelope.diagnostic.elapsed_ms,
                true,
                envelope
                    .diagnostic
                    .fallback
                    .map(Fallback::note)
                    .unwrap_or(""),
            );
            let profile = connection.client.identity();
            h.origin = TextEdit::new(&profile.0);
            h.installation = TextEdit::new(&profile.1);
            match connection.stored {
                Ok(true) => {
                    h.saved = Some(profile.clone());
                    if s.prefs.set(PROFILE, &profile).is_err() {
                        h.status = "Connected. The app couldn't save the connection label.".into();
                    }
                }
                Ok(false) => {}
                Err(message) => h.status = message,
            }
            // A new connection starts with fresh diagnostics; catalogue
            // diagnostics from this session are kept after later successes.
            h.last_failure = None;
            h.last_catalogue_failure = None;
            h.last_fallback = connection.fallback;
            h.library = Some(Library::new(connection.client, connection.page));
            if s.screen == Screen::Connection {
                s.screen = Screen::Hosted;
                tx.blur();
            }
            h.start_catalogue();
        }
        (Operation::LoadPage(position), Ok(Reply::Page(read))) => {
            h.log_event(
                &pending,
                Lane::Library,
                TrailKind::Completed,
                envelope.diagnostic.elapsed_ms,
                true,
                read.fallback.map(Fallback::note).unwrap_or(""),
            );
            if read.fallback.is_some() {
                h.last_fallback = read.fallback;
            }
            if let Some(library) = &mut h.library {
                library.show(read, position);
            }
        }
        (Operation::BrowseFolders, Ok(Reply::Folders(folders))) => {
            h.log_event(
                &pending,
                Lane::Library,
                TrailKind::Completed,
                envelope.diagnostic.elapsed_ms,
                true,
                "",
            );
            if let Some(library) = &mut h.library {
                library.folders = Some(folders);
            }
        }
        (Operation::Transfer(Transfer::Preview), Ok(Reply::Preview(bytes))) => {
            h.log_event(
                &pending,
                Lane::Library,
                TrailKind::Completed,
                envelope.diagnostic.elapsed_ms,
                true,
                "",
            );
            if let (Some(library), Some(cache)) = (&mut h.library, &s.cache) {
                library.preview = Some(cache.load(Arc::<[u8]>::from(bytes)));
            }
        }
        (Operation::Forget, Ok(Reply::Forgotten)) => {
            h.log_event(
                &pending,
                Lane::Library,
                TrailKind::Completed,
                envelope.diagnostic.elapsed_ms,
                true,
                "",
            );
            h.saved = None;
            h.status = if s.prefs.remove(PROFILE).is_ok() {
                "Saved key removed from this device.".into()
            } else {
                "The key was removed. The app couldn't clear the saved connection label.".into()
            };
        }
        (operation, Err(failure)) => {
            h.log_event(
                &pending,
                Lane::Library,
                TrailKind::Failed,
                envelope.diagnostic.elapsed_ms,
                true,
                &failure.cause(),
            );
            h.last_failure = Some(failure.clone());
            if failure.unauthorized && h.connected() {
                h.disconnect();
                if s.screen == Screen::Hosted {
                    s.screen = Screen::Connection;
                    tx.blur();
                }
            }
            h.status = failure.message;
            if matches!(operation, Operation::LoadPage(_)) && h.connected() {
                h.status.push_str(" Showing the previous view.");
            }
        }
        _ => h.status = "The library operation did not finish as expected. Try again.".into(),
    }
}

/// Optional discovery lands here, never through the library's `Latest`. It
/// checks the session and its own correlation before touching library state.
fn apply_catalogue(s: &mut Studio, tx: &mut Tx, envelope: CatalogueEnvelope) {
    let h = &mut s.hosted;
    let current = h.connected()
        && envelope.pending.session == h.session
        && h.catalogue_pending.as_ref().is_some_and(|pending| {
            pending.correlation == envelope.pending.correlation && pending.session == h.session
        })
        && h.library
            .as_ref()
            .is_some_and(|library| matches!(library.catalogue, Catalogue::Pending { .. }));
    if !current {
        h.log_event(
            &envelope.pending,
            Lane::Catalogue,
            TrailKind::Ignored,
            elapsed_ms(envelope.pending.started),
            false,
            "result belonged to an earlier connection; transport not known to be cancelled",
        );
        return;
    }
    let Some(pending) = h.catalogue_pending.take() else {
        return;
    };
    let elapsed = elapsed_ms(pending.started);
    match envelope.result {
        Ok(models) => {
            h.log_event(
                &pending,
                Lane::Catalogue,
                TrailKind::Completed,
                elapsed,
                true,
                "",
            );
            if let Some(library) = &mut h.library {
                library.catalogue = Catalogue::Ready(models);
            }
            if h.status.is_empty() {
                h.status = "Model catalogue ready.".into();
            }
        }
        Err(failure) => {
            h.log_event(
                &pending,
                Lane::Catalogue,
                TrailKind::Failed,
                elapsed,
                true,
                &failure.cause(),
            );
            h.last_catalogue_failure = Some(failure.clone());
            if let Some(library) = &mut h.library {
                library.catalogue = Catalogue::Failed(failure.clone());
            }
            if failure.unauthorized {
                // Shared authority is gone: both lanes and export protection
                // are invalidated by the same disconnect.
                h.disconnect();
                if s.screen == Screen::Hosted {
                    s.screen = Screen::Connection;
                    tx.blur();
                }
            }
            h.status = failure.message;
        }
    }
}

fn field(
    key: &str,
    name: &str,
    hint: &str,
    edit: &TextEdit,
    p: Ink,
    on_edit: fn(&mut Studio) -> &mut TextEdit,
) -> El {
    let input = text_input(edit, TextStyle::new(14.0, 450.0))
        .key(key.to_owned())
        .label(name.to_owned())
        .single_line()
        .w_full()
        .h(40.0)
        .padding(10.0)
        .radius(7.0)
        .fill(p.field)
        .focus_ring(2.0, 2.0, p.accent)
        .color(p.primary)
        .placeholder(hint.to_owned(), p.muted)
        .on_edit(on_edit);
    col()
        .w_full()
        .gap(5.0)
        .child(label(name, 13.0, 600.0, p.primary))
        .child(input)
}

fn status(h: &Hosted, p: Ink) -> El {
    let message: &str = match &h.operation {
        Some(operation) => operation.label(),
        None => &h.status,
    };
    label(message, 13.0, 500.0, p.accent).wrap()
}

/// Deliberate, local evidence. The same string shown for review is the one
/// `Tx::copy` writes; nothing is sent upstream or filed automatically.
fn report_panel(h: &Hosted, p: Ink) -> El {
    let mut panel = col()
        .key("connection-report")
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(8.0)
        .fill(p.panel)
        .border(1.0, p.line)
        .child(label("Connection report", 14.0, 650.0, p.primary))
        .child(label(h.summary(), 12.0, 500.0, p.muted).wrap())
        .child(
            row()
                .gap(7.0)
                .child(action(
                    "copy-report",
                    "Copy connection report",
                    p,
                    false,
                    |s, tx| {
                        let report = s.hosted.report();
                        tx.copy(report);
                        s.hosted.status =
                            "Connection report copied locally. Paste it into the bug form.".into();
                    },
                ))
                .child(action(
                    "report-details",
                    if h.report_expanded {
                        "Hide report details"
                    } else {
                        "Show report details"
                    },
                    p,
                    false,
                    |s, _| s.hosted.report_expanded = !s.hosted.report_expanded,
                )),
        );
    if h.report_expanded {
        // A short, independently scrollable preview: the review stays inside
        // the report card even when the inspector column is not scrolled.
        panel = panel.child(
            col()
                .key("connection-report-scroll")
                .w_full()
                .h(150.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(h.report())
                        .key("connection-report-text")
                        .font(TextStyle::new(11.0, 450.0))
                        .color(p.muted)
                        .wrap(),
                ),
        );
    }
    panel
}

pub(super) fn connection(s: &Studio, p: Ink) -> El {
    let h = &s.hosted;
    let title = col().gap(9.0)
        .child(label("Connect your Prints library", 34.0, 650.0, p.primary).wrap())
        .child(label("Open the library on your selected Oyster Computer. Afterlight stays available offline.", 15.0, 450.0, p.muted).wrap());
    let title = if !h.status.is_empty() || (h.busy() && h.connected()) {
        title.child(status(h, p))
    } else {
        title
    };
    let mut form = col()
        .w_full()
        .max_w(560.0)
        .gap(14.0)
        .padding(22.0)
        .radius(12.0)
        .fill(p.panel)
        .border(1.0, p.line);
    if let Some(library) = &h.library {
        let client = &library.client;
        form = form
            .child(label(client.identity().0, 14.0, 600.0, p.primary).wrap())
            .child(action("open-hosted", "Open library", p, true, |s, _| {
                s.screen = Screen::Hosted
            }))
            .child(action(
                "disconnect-hosted",
                "Disconnect",
                p,
                false,
                |s, _| s.hosted.disconnect(),
            ));
    } else if h.busy() {
        form = form.child(status(h, p));
    } else {
        form = form
            .child(field(
                "oyster-origin",
                "Oyster origin",
                "https://your-oyster.example",
                &h.origin,
                p,
                |s| &mut s.hosted.origin,
            ))
            .child(field(
                "prints-installation",
                "Prints installation ID",
                "prints",
                &h.installation,
                p,
                |s| &mut s.hosted.installation,
            ))
            .child(field(
                "prints-key",
                "Scoped App key",
                "Enter your named App key",
                &h.key,
                p,
                |s| &mut s.hosted.key,
            ))
            .child(action(
                "remember-key",
                if h.remember {
                    "Remember key on this device (on)"
                } else {
                    "Remember key on this device (off)"
                },
                p,
                false,
                |s, _| s.hosted.remember = !s.hosted.remember,
            ))
            .child(action(
                "connect-hosted",
                "Connect and open library",
                p,
                true,
                |s, _| s.hosted.connect(false),
            ));
        if h.saved.is_some() {
            form = form.child(
                row()
                    .gap(8.0)
                    .child(action(
                        "reconnect-hosted",
                        "Use saved connection",
                        p,
                        false,
                        |s, _| s.hosted.connect(true),
                    ))
                    .child(action(
                        "forget-hosted",
                        "Forget saved key",
                        p,
                        false,
                        |s, _| s.hosted.forget(),
                    )),
            );
        }
    }
    col()
        .key("connection-view")
        .w_full()
        .h_full()
        .grow(1.0)
        .padding(26.0)
        .gap(20.0)
        .child(title)
        .child(form)
        .child(report_panel(h, p))
        .child(
            label(
                "Use a named App key with apps.read for this Prints installation.",
                13.0,
                500.0,
                p.muted,
            )
            .wrap(),
        )
        .child(action(
            "setup-guide",
            "Read the Prints setup guide",
            p,
            false,
            |_, tx| {
                let _ = tx.open_url("https://prints.loftwah.com/guide#connect");
            },
        ))
}

fn asset_detail(h: &Hosted, library: &Library, item: &OutputItem, p: Ink) -> El {
    let dimensions = match (item.print.width, item.print.height) {
        (Some(width), Some(height)) => format!("{width} × {height}"),
        _ => item
            .print
            .duration_seconds
            .map(|s| format!("{s} seconds"))
            .unwrap_or_default(),
    };
    let mut detail = col()
        .w_full()
        .min_w(0.0)
        .gap(10.0)
        .child(label(filename(&item.path), 23.0, 650.0, p.primary).wrap())
        .child(
            label(
                format!(
                    "{} · {:.1} MiB · {dimensions}",
                    item.print.format.to_uppercase(),
                    item.print.bytes as f64 / (1024.0 * 1024.0)
                ),
                13.0,
                500.0,
                p.muted,
            )
            .wrap(),
        )
        .child(label(&item.print.model, 13.0, 500.0, p.muted).wrap());
    if !h.busy() {
        let mut buttons = row().gap(7.0);
        if item.print.bytes <= EXPORT_BYTES && !h.transfer_busy() {
            buttons = buttons.child(action("hosted-save", "Export original", p, true, |s, _| {
                s.hosted.transfer(Transfer::Original)
            }));
        }
        if item.print.kind == "image"
            && item.print.bytes <= PREVIEW_BYTES
            && library.preview.is_none()
        {
            buttons = buttons.child(action(
                "hosted-preview",
                "Load image preview",
                p,
                false,
                |s, _| s.hosted.transfer(Transfer::Preview),
            ));
        }
        if let Some(typed) = &item.typed {
            let prompt = typed.prompt.clone();
            buttons = buttons.child(action(
                "hosted-copy-prompt",
                "Copy prompt",
                p,
                false,
                move |s, tx| {
                    tx.copy(prompt.clone());
                    s.hosted.status = "Prompt copied.".into();
                },
            ));
        }
        detail = detail.child(buttons);
    }
    if let Some(image) = &library.preview {
        let mut panel = el().w_full().h(280.0).fill(p.stage).clip_rounded(9.0);
        panel = if image.is_ready() {
            panel.pixels_fit(image.pixels(), SurfaceFit::Fit)
        } else if image.error().is_some() {
            panel.center().child(
                label(
                    "Preview unavailable. You can still export the original.",
                    13.0,
                    500.0,
                    p.muted,
                )
                .wrap(),
            )
        } else {
            panel
                .center()
                .child(label("Decoding preview…", 13.0, 500.0, p.muted))
        };
        detail = detail.child(panel);
    }
    if let Some(style) = &item.style {
        detail = detail.child(label(format!("Style · {style}"), 13.0, 500.0, p.muted).wrap());
    }
    if !item.characters.is_empty() {
        detail = detail.child(
            label(
                format!("Characters · {}", item.characters.join(", ")),
                13.0,
                500.0,
                p.muted,
            )
            .wrap(),
        );
    }
    if let Some(typed) = &item.typed {
        detail = detail
            .child(label(&typed.prompt, 13.0, 450.0, p.primary).wrap())
            .child(label(
                format!("{} source references", typed.references),
                12.0,
                500.0,
                p.muted,
            ));
    }
    detail
}

pub(super) fn library(s: &Studio, p: Ink) -> El {
    let h = &s.hosted;
    let Some(library) = &h.library else {
        return connection(s, p);
    };
    let client = &library.client;
    let (origin, installation) = client.identity();
    let url = client.app_url();
    let title = row()
        .w_full()
        .items_center()
        .gap(8.0)
        .child(
            col()
                .grow(1.0)
                .gap(3.0)
                .child(label("Prints library", 34.0, 650.0, p.primary).wrap())
                .child(label(format!("{origin} · {installation}"), 13.0, 500.0, p.muted).wrap()),
        )
        .child(action(
            "open-web-prints",
            "Create in web Prints",
            p,
            false,
            move |_, tx| {
                let _ = tx.open_url(&url);
            },
        ));
    let filters = if client.root_only() {
        label(
            "This installation exposes its root library. Open web Prints to browse folders.",
            13.0,
            500.0,
            p.muted,
        )
        .wrap()
    } else {
        row()
            .w_full()
            .items_center()
            .gap(7.0)
            .child(
                field(
                    "library-folder",
                    "Library folder",
                    "All folders",
                    &h.folder,
                    p,
                    |s| &mut s.hosted.folder,
                )
                .grow(1.0)
                .min_w(0.0),
            )
            .child(action("library-apply", "Open folder", p, false, |s, _| {
                s.hosted.apply_folder()
            }))
            .child(action("library-root", "All folders", p, false, |s, _| {
                s.hosted.folder = TextEdit::new("");
                s.hosted.apply_folder();
            }))
    };
    let mut items = col()
        .w_full()
        .gap(5.0)
        .padding(9.0)
        .radius(10.0)
        .fill(p.panel);
    if !client.root_only() {
        items = items.child(action(
            "browse-folders",
            "Browse folders",
            p,
            false,
            |s, _| s.hosted.browse_folders(None),
        ));
    }
    if let Some(folders) = &library.folders {
        if folders.folders.is_empty() {
            items = items.child(label("No subfolders here.", 13.0, 500.0, p.muted));
        }
        for (index, folder) in folders.folders.iter().enumerate() {
            let path = folder.folder.clone();
            items = items.child(nav(
                &format!("folder-{index}"),
                &folder.name,
                false,
                p,
                move |s, _| {
                    s.hosted.folder = TextEdit::new(&path);
                    s.hosted.apply_folder();
                },
            ));
        }
        if let Some(next) = folders.next {
            items = items.child(action(
                "more-folders",
                "More folders",
                p,
                false,
                move |s, _| s.hosted.browse_folders(Some(next)),
            ));
        }
    }
    if library.page.items.is_empty() {
        items =
            items.child(label("This folder has no indexed prints.", 13.0, 500.0, p.muted).wrap());
    }
    for (index, item) in library.page.items.iter().enumerate() {
        let name = filename(&item.path);
        items = items.child(nav(
            &format!("hosted-item-{index}"),
            &name,
            library.selected == Some(index),
            p,
            move |s, _| s.hosted.select(index),
        ));
    }
    if !h.busy() {
        let mut pagination = row().gap(4.0);
        if library.position.index > 0 {
            pagination = pagination.child(action("hosted-prev", "Previous", p, false, |s, _| {
                if let Some(previous) = s
                    .hosted
                    .library
                    .as_ref()
                    .and_then(|library| library.position.previous())
                {
                    s.hosted.load_page(previous);
                }
            }));
        }
        if library.page.next.is_some() {
            pagination = pagination.child(action("hosted-next", "Next", p, false, |s, _| {
                let next = s.hosted.library.as_ref().and_then(|library| {
                    library
                        .page
                        .next
                        .clone()
                        .map(|cursor| library.position.next(cursor))
                });
                if let Some(next) = next {
                    s.hosted.load_page(next);
                }
            }));
        }
        items = items.child(pagination);
    }
    let mut detail = col().w_full().min_w(0.0).gap(12.0);
    if let Some(item) = library.selected() {
        detail = detail.child(asset_detail(h, library, item, p));
    }
    let mut catalogue = col()
        .w_full()
        .gap(5.0)
        .padding(12.0)
        .radius(8.0)
        .fill(p.panel);
    match &library.catalogue {
        Catalogue::Pending { retry } => {
            catalogue = catalogue.child(
                label(
                    "Discovering models… Optional. Browsing, preview and export stay available.",
                    13.0,
                    500.0,
                    p.muted,
                )
                .wrap(),
            );
            if *retry > 0 {
                catalogue = catalogue.child(label(
                    format!("Discovery retry {retry}."),
                    12.0,
                    500.0,
                    p.muted,
                ));
            }
        }
        Catalogue::Ready(models) => {
            catalogue = catalogue.child(label(
                format!("Available models · {}", models.len()),
                14.0,
                650.0,
                p.primary,
            ));
            for model in models {
                let title = model.label.as_deref().unwrap_or(&model.id);
                let kind = model.kind.as_deref().unwrap_or("Media");
                catalogue = catalogue
                    .child(label(format!("{title} · {kind}"), 12.0, 500.0, p.muted).wrap());
            }
        }
        Catalogue::Failed(failure) => {
            catalogue = catalogue
                .child(
                    label(
                        "Model catalogue unavailable for this connection.",
                        13.0,
                        600.0,
                        p.primary,
                    )
                    .wrap(),
                )
                .child(label(&failure.message, 12.0, 500.0, p.muted).wrap())
                .child(label(failure.recovery, 12.0, 500.0, p.muted).wrap())
                .child(action(
                    "catalogue-retry",
                    "Retry model discovery",
                    p,
                    false,
                    |s, _| s.hosted.retry_catalogue(),
                ));
        }
    }
    detail = detail.child(catalogue).child(report_panel(h, p));
    let status_row = row()
        .w_full()
        .gap(10.0)
        .items_center()
        .child(status(h, p).grow(1.0).shrink().min_w(0.0));
    let mut status_row = if let Some(path) = &library.last_export {
        let path = path.clone();
        status_row.child(action(
            "hosted-reveal",
            "Show last export",
            p,
            false,
            move |s, tx| {
                if tx.reveal_path(&path).is_err() {
                    s.hosted.status = "Couldn't show that file in the desktop file manager.".into();
                }
            },
        ))
    } else {
        status_row
    };
    let transfer_feedback = match &h.transfer {
        TransferState::Fetching {
            observed, total, ..
        } => {
            let progress = match total {
                Some(total) => format!("Fetching original · {observed} of {total} bytes received"),
                None => format!("Fetching original · {observed} bytes received · total unknown"),
            };
            Some(
                row()
                    .w_full()
                    .gap(10.0)
                    .items_center()
                    .child(
                        label(progress, 12.0, 500.0, p.muted)
                            .wrap()
                            .grow(1.0)
                            .shrink()
                            .min_w(0.0),
                    )
                    .child(action(
                        "hosted-transfer-cancel",
                        "Stop transfer",
                        p,
                        false,
                        |s, _| s.hosted.cancel_transfer(),
                    )),
            )
        }
        TransferState::Failed { .. } => {
            status_row = status_row.child(action(
                "hosted-transfer-retry",
                "Retry transfer",
                p,
                false,
                |s, _| s.hosted.retry_transfer(),
            ));
            None
        }
        _ => None,
    };
    let feedback = col()
        .w_full()
        .gap(5.0)
        .child(status_row)
        .children(transfer_feedback);
    col()
        .key("hosted-view")
        .w_full()
        .h_full()
        .grow(1.0)
        .padding(22.0)
        .gap(13.0)
        .child(title)
        .child(filters)
        .child(feedback)
        .child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .gap(14.0)
                .child(
                    col()
                        .w(245.0)
                        .h_full()
                        .min_h(0.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(items),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .h_full()
                        .min_h(0.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(detail),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(correlation: &str) -> Pending {
        Pending {
            correlation: correlation.into(),
            session: 1,
            started: Instant::now(),
            method: "listModels",
        }
    }

    #[test]
    fn report_keeps_typed_fields_and_excludes_injected_values() {
        let mut h = Hosted::new(&Prefs::memory());
        h.last_failure = Some(Failure::from_error(
            "connect",
            "listOutputs",
            Error::Status {
                status: 502,
                code: Some("oyk_injected_key".into()),
            },
            37,
            Some(Fallback::RecursiveReadUnavailable),
        ));
        h.last_catalogue_failure = Some(Failure::catalogue(
            Error::Refused {
                method: "listModels",
                code: "not_granted".into(),
            },
            19,
        ));
        h.log_event(
            &pending("op-7"),
            Lane::Catalogue,
            TrailKind::Failed,
            19,
            true,
            "method refused",
        );
        let report = h.report();
        assert!(report.contains("Prints Desktop connection report"));
        assert!(report.contains(env!("CARGO_PKG_VERSION")));
        assert!(report.contains(std::env::consts::OS));
        assert!(report.contains(std::env::consts::ARCH));
        assert!(report.contains("http 502"));
        assert!(report.contains("code=not_granted"));
        assert!(report.contains("37ms"));
        assert!(report.contains("root read attempted after recursive read failed"));
        assert!(report.contains("op-7"));
        assert!(report.contains("session invalidated") == false);
        for secret in [
            "oyk_injected_key",
            "Authorization:",
            "/library/Studies/horizon.jpg",
            "fictional-computer",
            "A fictional landscape from a local fixture.",
        ] {
            assert!(!report.contains(secret), "report leaked {secret}");
        }
        // A catalogue failure stays in the summary even after the library read
        // it accompanied succeeded.
        assert!(h.summary().contains("media.generation"));
    }

    #[test]
    fn trail_stays_bounded_with_opaque_ids() {
        let mut h = Hosted::new(&Prefs::memory());
        for index in 0..24 {
            h.log_event(
                &pending(&format!("op-{index}")),
                Lane::Library,
                TrailKind::Completed,
                index,
                true,
                "",
            );
        }
        assert_eq!(h.trail.len(), TRAIL_LIMIT);
        let report = h.report();
        assert!(!report.contains("op-0 "));
        assert!(report.contains("op-14 "));
    }

    fn hosted_with_one_item() -> Hosted {
        let item: OutputItem = serde_json::from_value(serde_json::json!({
            "print": {
                "printId": "print_abc",
                "computerId": "computer-1",
                "kind": "image",
                "model": "fixture",
                "format": "png",
                "bytes": 3,
                "createdAt": 1730000000000u64,
            },
            "path": "/prints/print.png",
        }))
        .unwrap();
        let client = OysterClient::new(
            "http://127.0.0.1:9",
            "prints",
            ApiKey::new("oyk_local_fixture_key"),
        )
        .unwrap();
        let mut h = Hosted::new(&Prefs::memory());
        h.library = Some(Library::new(
            Arc::new(client),
            OutputRead {
                page: OutputPage {
                    items: vec![item],
                    next: None,
                },
                fallback: None,
            },
        ));
        h
    }

    #[test]
    fn original_transfer_owns_its_own_lane_and_allows_one_at_a_time() {
        let mut h = hosted_with_one_item();
        assert!(h.library.as_ref().is_some_and(|l| l.selected().is_some()));
        h.transfer(Transfer::Original);
        assert!(!h.busy(), "browsing, selection and preview stay available");
        assert!(
            h.transfer_busy(),
            "a second transfer must not queue a dialog"
        );
        assert!(matches!(
            &h.transfer,
            TransferState::Fetching { observed, total, .. }
                if *observed == 0 && *total == Some(3)
        ));
        h.transfer(Transfer::Original);
        assert!(matches!(&h.transfer, TransferState::Fetching { .. }));
    }

    #[test]
    fn stopping_a_transfer_has_no_library_effect_and_a_redacted_trail() {
        let mut h = hosted_with_one_item();
        h.transfer(Transfer::Original);
        h.cancel_transfer();
        assert!(matches!(&h.transfer, TransferState::Stopping { .. }));
        assert!(h.transfer_busy());
        assert!(!h.busy());
        assert!(h.library.is_some(), "the last readable page is unchanged");
        let report = h.report();
        assert!(report.contains("transfer openOutput"));
        assert!(report.contains("in-flight request not known to be cancelled"));
        assert!(!report.contains("/prints/print.png"));
        assert!(!report.contains("oyk_local_fixture_key"));
    }

    #[test]
    fn retry_rebinds_capture_to_current_generation_and_new_correlation() {
        let mut h = hosted_with_one_item();
        let other: OutputItem = serde_json::from_value(serde_json::json!({
            "print": {
                "printId": "print_other",
                "computerId": "computer-1",
                "kind": "image",
                "model": "fixture",
                "format": "png",
                "bytes": 7,
                "createdAt": 1730000000001u64,
            },
            "path": "/prints/other.png",
        }))
        .unwrap();
        let library = h.library.as_mut().unwrap();
        library.page.items.push(other);
        library.selected = Some(1);

        // The failed attempt was sealed to an older correlation.
        let original = library.page.items[0].clone();
        let old_capture = TransferCapture::new(&original, h.epoch, "op-old");
        let old_seal = old_capture.seal();
        h.transfer = TransferState::Failed {
            capture: old_capture,
        };

        h.retry_transfer();

        match &h.transfer {
            TransferState::Fetching {
                capture, pending, ..
            } => {
                assert_ne!(pending.correlation, "op-old");
                assert_eq!(pending.seal, capture.seal());
                assert!(capture.is_sealed_by(pending.epoch, &pending.correlation));
                assert_ne!(
                    capture.seal(),
                    old_seal,
                    "retry must re-seal for its own request"
                );
                assert_eq!(capture.path(), "/prints/print.png");
                assert_eq!(capture.item().print.print_id, "print_abc");
            }
            _ => panic!("retry did not start a new fetch"),
        }
    }

    #[test]
    fn save_dialog_owns_its_correlation_without_superseding_library_work() {
        let mut h = hosted_with_one_item();
        let page_pending = Pending {
            correlation: "op-page".into(),
            session: h.session,
            started: Instant::now(),
            method: "listOutputs",
        };
        h.primary_pending = Some(page_pending);
        h.operation = Some(Operation::Transfer(Transfer::Preview));
        h.transfer(Transfer::Original);
        let (capture, pending) = match &h.transfer {
            TransferState::Fetching {
                capture, pending, ..
            } => (capture.clone(), pending.clone()),
            _ => panic!("fetch did not start"),
        };
        let envelope = TransferEnvelope {
            pending: pending.clone(),
            diagnostic: Diagnostic::success("fetch original", "openOutput", 1, None),
            result: Ok(TransferReply::Original(b"abc".to_vec())),
        };
        let mut s = Studio::new();
        s.screen = Screen::Hosted;
        s.hosted = h;
        let mut tx = Tx::default();
        apply_transfer(&mut s, &mut tx, envelope);

        // Opening the export dialog must not invalidate an in-flight reply.
        assert!(
            s.hosted
                .primary_pending
                .as_ref()
                .is_some_and(|pending| pending.correlation == "op-page")
        );
        assert!(matches!(
            s.hosted.operation,
            Some(Operation::Transfer(Transfer::Preview))
        ));
        let dialog = match &s.hosted.transfer {
            TransferState::Dialog { dialog, .. } => dialog.clone(),
            _ => panic!("expected a dialog owned by the transfer lane"),
        };
        assert_ne!(
            dialog.correlation, pending.correlation,
            "the dialog needs its own correlation"
        );
        let (epoch, session, seal) = (s.hosted.epoch, s.hosted.session, capture.seal());
        assert!(dialog_is_current(
            &s.hosted,
            epoch,
            session,
            seal,
            &dialog.correlation
        ));
        assert!(!dialog_is_current(
            &s.hosted,
            epoch,
            session,
            seal,
            &pending.correlation
        ));
    }

    #[test]
    fn revoked_catalogue_failure_asks_to_reconnect_without_claiming_library_available() {
        let revoked = Failure::catalogue(
            Error::Status {
                status: 401,
                code: None,
            },
            5,
        );
        assert!(revoked.unauthorized);
        assert!(!revoked.message.contains("still available"));
        assert!(revoked.recovery.contains("Reconnect"));

        let denied = Failure::catalogue(
            Error::Refused {
                method: "listModels",
                code: "not_granted".into(),
            },
            5,
        );
        assert!(!denied.unauthorized);
        assert!(denied.message.contains("still available"));
        assert!(denied.recovery.contains("Grant access"));
    }
}
