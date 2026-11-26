//! Read-only Oyster / Prints client for Prints Desktop.
//!
//! Contract and source references: `docs/oyster.md`. This module implements
//! only the documented typed HTTP route
//! `POST {origin}/apps/<installationId>/methods/<name>` with a named `oyk_…`
//! App key sent as `Authorization: Bearer …`. It deliberately does **not**
//! implement MCP `execute` (a JavaScript string surface), generation, key
//! storage, or any write method.
//!
#![forbid(unsafe_code)]

use std::fmt;
use std::io::Read;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

mod library;
pub use library::{Folder, FolderPage, TransferCapture};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

/// Largest JSON reply accepted from any method call. A `listModels` page holds
/// at most 128 models and a `listOutputs` page at most 24 items, so this is
/// generous while still bounded.
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Prints' `openOutput` maximum and default `byteLength`.
const MAX_CHUNK_BYTES: usize = 128 * 1024;
/// Local safety bound for assembled original bytes (web Prints caps downloads
/// at 32 MiB; this is a client limit, not a server guarantee).
const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;
/// Maximum base64 `content` characters in `byteRead` (`method-contracts.ts`).
const MAX_CONTENT_CHARS: usize = 174_764;
const TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_BUDGET: Duration = Duration::from_secs(75);

/// A named Oyster key. Its text is never shown by `Debug` or `Display`, never
/// put in a URL, and never copied into an error.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wrap a key supplied by the app's credential integration. This module
    /// does not read it from anywhere.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    fn is_named(&self) -> bool {
        self.0.strip_prefix("oyk_").is_some_and(|suffix| {
            (8..=256).contains(&suffix.len())
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        })
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey([REDACTED])")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Secret-free failure values. Server `message` text is intentionally dropped;
/// only closed-grammar codes and HTTP statuses are retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidBaseUrl,
    CredentialsInUrl,
    InsecureBaseUrl,
    InvalidInstallationId,
    InvalidKey,
    InvalidFolder,
    InvalidPrintId,
    InvalidArgument,
    Transport,
    Status {
        status: u16,
        code: Option<String>,
    },
    Refused {
        method: &'static str,
        code: String,
    },
    TooLarge,
    Decode,
    /// A cooperative stop observed before a further chunk was requested. It
    /// never claims that an already in-flight blocking call was aborted.
    Cancelled,
}

impl Error {
    /// Distinguish lost connection authority from a denied individual method.
    pub fn authority_lost(&self) -> bool {
        let code = match self {
            Self::Status { status: 401, .. } => return true,
            Self::Status { code, .. } => code.as_deref(),
            Self::Refused { code, .. } => Some(code.as_str()),
            _ => None,
        };
        matches!(
            code,
            Some("revoked" | "stale_authority" | "grantor_lost_authority")
        )
    }
}

/// Closed failure categories used by the local connection report. Server text
/// is never copied into one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Success,
    Transport,
    Status,
    Refused,
    TooLarge,
    InvalidArgument,
    Decode,
    Local,
    Cancelled,
}

impl Category {
    /// Stable, lowercase wording suitable for the bug form.
    pub fn label(self) -> &'static str {
        match self {
            Self::Success => "completed",
            Self::Transport => "transport failure",
            Self::Status => "http status",
            Self::Refused => "method refused",
            Self::TooLarge => "size limit",
            Self::InvalidArgument => "invalid local argument",
            Self::Decode => "malformed reply",
            Self::Local => "local operation failure",
            Self::Cancelled => "transfer stopped before next chunk",
        }
    }
}

/// A compatibility behaviour the client actually observed. The text is fixed
/// in this crate; no host-supplied message can enter the report through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    RecursiveReadUnavailable,
    RootOnlyDeclared,
}

impl Fallback {
    pub fn note(self) -> &'static str {
        match self {
            Self::RecursiveReadUnavailable => "root read attempted after recursive read failed",
            Self::RootOnlyDeclared => "root-only library contract confirmed",
        }
    }
}

/// Secret-free, bounded evidence for one typed call. Both successful and
/// failed calls produce one, so a partial capability failure stays inspectable
/// after the required library read succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub operation: &'static str,
    pub method: &'static str,
    pub category: Category,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub elapsed_ms: u64,
    pub fallback: Option<Fallback>,
}

impl Diagnostic {
    pub fn success(
        operation: &'static str,
        method: &'static str,
        elapsed_ms: u64,
        fallback: Option<Fallback>,
    ) -> Self {
        Self {
            operation,
            method,
            category: Category::Success,
            status: None,
            code: None,
            elapsed_ms,
            fallback,
        }
    }
}

impl Error {
    pub fn category(&self) -> Category {
        match self {
            Self::Transport => Category::Transport,
            Self::Status { .. } => Category::Status,
            Self::Refused { .. } => Category::Refused,
            Self::TooLarge => Category::TooLarge,
            Self::Cancelled => Category::Cancelled,
            Self::InvalidArgument
            | Self::InvalidBaseUrl
            | Self::CredentialsInUrl
            | Self::InsecureBaseUrl
            | Self::InvalidInstallationId
            | Self::InvalidKey
            | Self::InvalidFolder
            | Self::InvalidPrintId => Category::InvalidArgument,
            Self::Decode => Category::Decode,
        }
    }

    /// Build report evidence. Even a caller-supplied code is passed through
    /// the refusal allowlist, so an echoed secret can never reach a report.
    pub fn diagnostic(
        &self,
        operation: &'static str,
        method: &'static str,
        elapsed_ms: u64,
        fallback: Option<Fallback>,
    ) -> Diagnostic {
        let (status, code) = match self {
            Self::Status { status, code } => {
                (Some(*status), code.as_deref().and_then(allowlisted_code))
            }
            Self::Refused { code, .. } => (None, allowlisted_code(code)),
            _ => (None, None),
        };
        Diagnostic {
            operation,
            method,
            category: self.category(),
            status,
            code: code.map(str::to_owned),
            elapsed_ms,
            fallback,
        }
    }
}

/// A failed library read plus the fallback it had attempted. The original
/// `list_outputs` API stays unchanged for the diagnostic example and tests.
#[derive(Debug)]
pub struct ReadFailure {
    pub error: Error,
    pub fallback: Option<Fallback>,
}

impl From<ReadFailure> for Error {
    fn from(failure: ReadFailure) -> Self {
        failure.error
    }
}

impl fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for ReadFailure {}

/// A library page and any compatibility fallback observed while reading it.
#[derive(Debug)]
pub struct OutputRead {
    pub page: OutputPage,
    pub fallback: Option<Fallback>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBaseUrl => f.write_str("invalid Oyster base URL"),
            Self::CredentialsInUrl => f.write_str("credentials in base URL are not allowed"),
            Self::InsecureBaseUrl => f.write_str("HTTPS is required except for a loopback Oyster"),
            Self::InvalidInstallationId => f.write_str("invalid Prints installation id"),
            Self::InvalidKey => f.write_str("missing or invalid App key"),
            Self::InvalidFolder => f.write_str("invalid library folder"),
            Self::InvalidPrintId => f.write_str("invalid print id"),
            Self::InvalidArgument => f.write_str("invalid argument"),
            Self::Transport => f.write_str("Oyster request failed"),
            Self::Status { status, code } => {
                write!(f, "Oyster returned HTTP {status}")?;
                if let Some(code) = code {
                    write!(f, " ({code})")?;
                }
                Ok(())
            }
            Self::Refused { method, code } => {
                write!(f, "Prints refused {method}: {code}")
            }
            Self::TooLarge => f.write_str("response exceeded the client byte limit"),
            Self::Decode => f.write_str("unexpected or malformed Oyster reply"),
            Self::Cancelled => f.write_str("transfer stopped before the next chunk was requested"),
        }
    }
}

impl std::error::Error for Error {}

/// One admitted, read-only connection to one Prints installation.
#[derive(Debug)]
pub struct OysterClient {
    base: Url,
    installation: String,
    key: ApiKey,
    http: Client,
    root_only: AtomicBool,
}

impl OysterClient {
    /// Validate the origin and installation id, then build a redirect-free,
    /// TLS-only blocking client. Nothing is persisted.
    pub fn new(base: &str, installation: &str, key: ApiKey) -> Result<Self, Error> {
        let base = validated_base(base)?;
        if !valid_installation(installation) {
            return Err(Error::InvalidInstallationId);
        }
        if !key.is_named() {
            return Err(Error::InvalidKey);
        }
        let http = Client::builder()
            .use_rustls_tls()
            .redirect(Policy::none())
            .timeout(TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|_| Error::Transport)?;
        Ok(Self {
            base,
            installation: installation.to_owned(),
            key,
            http,
            root_only: AtomicBool::new(false),
        })
    }

    /// Canonical origin and the validated installation identifier.
    pub fn identity(&self) -> (String, String) {
        (
            self.base.origin().ascii_serialization(),
            self.installation.clone(),
        )
    }

    /// Browser entry point, without a key or private asset path.
    pub fn app_url(&self) -> String {
        let mut url = self.base.clone();
        url.set_path(&format!("/apps/{}/", self.installation));
        url.into()
    }

    /// `test-connection`: `listModels` is a read that spends nothing.
    pub fn test_connection(&self) -> Result<ModelList, Error> {
        self.list_models()
    }

    /// `listModels` — requires the installation's approved `media.generation`.
    pub fn list_models(&self) -> Result<ModelList, Error> {
        let value = self.call("listModels", json!({}))?;
        serde_json::from_value(value).map_err(|_| Error::Decode)
    }

    /// This connection accepted the root-only library contract.
    pub fn root_only(&self) -> bool {
        self.root_only.load(Ordering::Relaxed)
    }

    /// `listOutputs` — requires approved `computer.list`. Pass `folder` as a
    /// relative library path (omit for the root) and `after` from a prior page.
    pub fn list_outputs(
        &self,
        folder: Option<&str>,
        recursive: bool,
        after: Option<Cursor>,
        limit: u8,
    ) -> Result<OutputPage, Error> {
        self.list_outputs_detailed(folder, recursive, after, limit)
            .map(|read| read.page)
            .map_err(Error::from)
    }

    /// The same read with the observed compatibility fallback retained for
    /// the local report. A failed retry still reports the attempted fallback.
    pub fn list_outputs_detailed(
        &self,
        folder: Option<&str>,
        recursive: bool,
        after: Option<Cursor>,
        limit: u8,
    ) -> Result<OutputRead, ReadFailure> {
        if !(1..=24).contains(&limit) {
            return Err(ReadFailure {
                error: Error::InvalidArgument,
                fallback: None,
            });
        }
        if let Some(folder) = folder {
            if self.root_only() || !valid_folder(folder) {
                return Err(ReadFailure {
                    error: Error::InvalidFolder,
                    fallback: None,
                });
            }
        }
        let root_only = self.root_only();
        let mut input = json!({ "limit": limit });
        if !root_only {
            input["recursive"] = json!(recursive);
        }
        if let Some(folder) = folder {
            input["folder"] = Value::String(folder.to_owned());
        }
        if let Some(cursor) = after {
            input["after"] = json!({
                "createdAt": cursor.created_at,
                "printId": cursor.print_id,
            });
        }
        let mut fallback = None;
        let result = self.call("listOutputs", input.clone());
        if let Err(error) = &result
            && !root_only
            && folder.is_none()
            && (matches!(
                error,
                Error::Status {
                    status: 400 | 502,
                    ..
                }
            ) || matches!(error, Error::Refused { code, .. } if code == "invalid_input"))
        {
            // Display the root page even if the recursive read is temporarily
            // unavailable. Only a host that explicitly rejects the folder
            // method confirms the older contract.
            fallback = Some(Fallback::RecursiveReadUnavailable);
            if let Some(object) = input.as_object_mut() {
                object.remove("recursive");
            }
            let page = serde_json::from_value(
                self.call("listOutputs", input)
                    .map_err(|error| ReadFailure { error, fallback })?,
            )
            .map_err(|_| ReadFailure {
                error: Error::Decode,
                fallback,
            })?;
            match self.call("listFolders", json!({ "limit": 1 })) {
                Err(Error::Status {
                    status: 403,
                    code: Some(code),
                }) if code == "not_declared" => {
                    self.root_only.store(true, Ordering::Relaxed);
                    fallback = Some(Fallback::RootOnlyDeclared);
                }
                Err(error) if error.authority_lost() => {
                    return Err(ReadFailure { error, fallback });
                }
                _ => {}
            }
            return Ok(OutputRead { page, fallback });
        }
        let page = serde_json::from_value(result.map_err(|error| ReadFailure { error, fallback })?)
            .map_err(|_| ReadFailure {
                error: Error::Decode,
                fallback,
            })?;
        Ok(OutputRead { page, fallback })
    }

    /// One base64 chunk of an indexed output — requires approved
    /// `computer.readFile`. `byteLength` may be 1..=131072.
    pub fn open_output_chunk(
        &self,
        print_id: &str,
        byte_offset: u64,
        byte_length: usize,
    ) -> Result<ByteRead, Error> {
        if !valid_print_id(print_id) {
            return Err(Error::InvalidPrintId);
        }
        if !(1..=MAX_CHUNK_BYTES).contains(&byte_length) {
            return Err(Error::InvalidArgument);
        }
        let value = self.call(
            "openOutput",
            json!({
                "printId": print_id,
                "byteOffset": byte_offset,
                "byteLength": byte_length,
            }),
        )?;
        let read: ByteRead = serde_json::from_value(value).map_err(|_| Error::Decode)?;
        if read.print_id != print_id
            || read.byte_offset != byte_offset
            || read.content.len() > MAX_CONTENT_CHARS
            || read.encoding != "base64"
            || !read.path.starts_with('/')
        {
            return Err(Error::Decode);
        }
        Ok(read)
    }

    /// Assemble the original bytes of one output. Stop on the **empty final
    /// slice**; `truncated` is true even on the last non-empty slice and must
    /// not be used as the stop condition (official Fungi byte-paging guide).
    pub fn download_output(&self, print_id: &str) -> Result<Vec<u8>, Error> {
        self.download_matching_output(print_id, None)
    }

    fn download_matching_output(
        &self,
        print_id: &str,
        expected_path: Option<&str>,
    ) -> Result<Vec<u8>, Error> {
        self.download_observed(print_id, expected_path, None, None, None)
    }

    /// The shared chunk loop. `cancel` is checked before each new request and
    /// again once a reply has been decoded: a stop therefore prevents further
    /// dispatch and drops the decoder buffer, while an already in-flight
    /// blocking call is left to finish without any claim that it was aborted.
    /// Progress is only reported from decoded, retained bytes, and `total` is
    /// passed through only when the caller has an explicit trustworthy value.
    pub(crate) fn download_observed(
        &self,
        print_id: &str,
        expected_path: Option<&str>,
        total: Option<u64>,
        cancel: Option<&AtomicBool>,
        mut on_progress: Option<&mut dyn FnMut(TransferProgress)>,
    ) -> Result<Vec<u8>, Error> {
        if !valid_print_id(print_id) {
            return Err(Error::InvalidPrintId);
        }
        let mut out = Vec::new();
        let mut offset: u64 = 0;
        let mut path = expected_path.map(str::to_owned);
        let deadline = Instant::now() + DOWNLOAD_BUDGET;
        for _ in 0..=MAX_DOWNLOAD_BYTES / MAX_CHUNK_BYTES + 16 {
            if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                out.clear();
                return Err(Error::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(Error::Transport);
            }
            let read = self.open_output_chunk(print_id, offset, MAX_CHUNK_BYTES)?;
            // The request above may already have been in flight when stop was
            // requested. Its bytes are dropped, not reported as a completion.
            if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                out.clear();
                return Err(Error::Cancelled);
            }
            match &path {
                None => path = Some(read.path.clone()),
                Some(expected) if expected == &read.path => {}
                Some(_) => return Err(Error::Decode),
            }
            if read.content.is_empty() {
                return Ok(out);
            }
            let bytes = STANDARD.decode(&read.content).map_err(|_| Error::Decode)?;
            if bytes.is_empty() || bytes.len() > MAX_CHUNK_BYTES {
                return Err(Error::Decode);
            }
            let new_len = out.len().saturating_add(bytes.len());
            if new_len > MAX_DOWNLOAD_BYTES {
                return Err(Error::TooLarge);
            }
            if total.is_some_and(|expected| new_len as u64 > expected) {
                return Err(Error::Decode);
            }
            out.extend_from_slice(&bytes);
            offset += bytes.len() as u64;
            if let Some(report) = on_progress.as_deref_mut() {
                report(TransferProgress {
                    observed: offset,
                    total,
                });
            }
        }
        Err(Error::TooLarge)
    }

    /// One fixed typed call. The method name is a `&'static str`, never caller
    /// text; the input is a JSON object; no flexible `execute` program exists.
    fn call(&self, method: &'static str, input: Value) -> Result<Value, Error> {
        let body = serde_json::to_vec(&input).map_err(|_| Error::Decode)?;
        let mut auth = HeaderValue::from_str(&format!("Bearer {}", self.key.0))
            .map_err(|_| Error::InvalidKey)?;
        auth.set_sensitive(true);
        let response = self
            .http
            .post(self.method_url(method))
            .header(AUTHORIZATION, auth)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .map_err(|_| Error::Transport)?;
        let status = response.status().as_u16();
        let bytes = read_bounded(response)?;
        let value = serde_json::from_slice::<Value>(&bytes);
        if !(200..300).contains(&status) {
            let code = value
                .ok()
                .as_ref()
                .and_then(refusal_code)
                .map(str::to_owned);
            return Err(Error::Status { status, code });
        }
        let value = value.map_err(|_| Error::Decode)?;
        if let Some(code) = refusal_code(&value) {
            // Prints' method-level refusals ride inside a 2xx value.
            return Err(Error::Refused {
                method,
                code: code.to_owned(),
            });
        }
        Ok(value)
    }

    fn method_url(&self, method: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(&format!("/apps/{}/methods/{}", self.installation, method));
        url.set_query(None);
        url.set_fragment(None);
        url
    }
}

/// Accept an HTTPS origin, or plain HTTP on loopback for
/// the `oyster try` local host. Reduce the supplied base to its origin so no
/// path, query or fragment can alter method URLs.
fn validated_base(raw: &str) -> Result<Url, Error> {
    let mut url = Url::parse(raw).map_err(|_| Error::InvalidBaseUrl)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::CredentialsInUrl);
    }
    let host = url.host_str().unwrap_or("");
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<Ipv4Addr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
        || host
            .trim_matches(['[', ']'])
            .parse::<Ipv6Addr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(Error::InsecureBaseUrl),
    }
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    url.set_username("").map_err(|_| Error::InvalidBaseUrl)?;
    url.set_password(None).map_err(|_| Error::InvalidBaseUrl)?;
    Ok(url)
}

/// `^[a-z][a-z0-9-]{0,62}$`, the host's own installation id grammar
/// (`oyster/src/host.ts:556-560`).
fn valid_installation(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Mirror `prints/src/library-view.ts:15-34`. An absent folder is the root;
/// an empty string is not a valid folder.
fn valid_folder(folder: &str) -> bool {
    if folder.is_empty() || folder.chars().count() > 4096 || folder.contains('\0') {
        return false;
    }
    folder.split('/').all(|segment| {
        !segment.is_empty() && segment != "." && segment != ".." && segment.len() <= 255
    })
}

fn valid_print_id(id: &str) -> bool {
    let count = id.chars().count();
    (1..=256).contains(&count) && !id.contains('\0') && !id.contains('/')
}

/// The host's closed refusal-code grammar. Nothing outside this list is ever
/// retained as a code, including an echoed key or other injected string.
const REFUSAL_CODES: &[&str] = &[
    "not_granted",
    "revoked",
    "grantor_lost_authority",
    "unavailable",
    "invalid",
    "too_large",
    "conflict",
    "invalid_path",
    "not_found",
    "library_unavailable",
    "attachment_budget_exceeded",
    "not_approved",
    "level_too_low",
    "requires_not_granted",
    "not_declared",
    "invalid_input",
    "stale_authority",
    "approval_required",
];

fn allowlisted_code(code: &str) -> Option<&'static str> {
    REFUSAL_CODES.iter().copied().find(|known| *known == code)
}

fn refusal_code(value: &Value) -> Option<&str> {
    let error = value.as_object()?.get("error")?;
    Some(match error.as_str().and_then(allowlisted_code) {
        Some(code) => code,
        None => "unknown_refusal",
    })
}

fn read_bounded(response: Response) -> Result<Vec<u8>, Error> {
    let mut buffer = Vec::new();
    let mut limited = response.take(MAX_RESPONSE_BYTES.saturating_add(1) as u64);
    limited
        .read_to_end(&mut buffer)
        .map_err(|_| Error::Transport)?;
    if buffer.len() > MAX_RESPONSE_BYTES {
        return Err(Error::TooLarge);
    }
    Ok(buffer)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelList {
    pub models: Vec<Model>,
}

/// Only the grounded fields (`model.id`, `model.label`); other model catalogue
/// fields are ignored rather than invented here.
#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub id: String,
    pub label: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cursor {
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "printId")]
    pub print_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputPage {
    pub items: Vec<OutputItem>,
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputItem {
    pub print: PrintRecord,
    /// Absolute path inside the admitted Computer's Prints library.
    pub path: String,
    #[serde(default)]
    pub characters: Vec<String>,
    pub style: Option<String>,
    pub typed: Option<TypedPrompt>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TypedPrompt {
    pub prompt: String,
    pub references: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrintRecord {
    #[serde(rename = "printId")]
    pub print_id: String,
    #[serde(rename = "computerId")]
    pub computer_id: String,
    pub kind: String,
    pub model: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(rename = "durationSeconds")]
    pub duration_seconds: Option<u32>,
    pub format: String,
    pub bytes: u64,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ByteRead {
    #[serde(rename = "printId")]
    pub print_id: String,
    pub path: String,
    #[serde(rename = "byteOffset")]
    pub byte_offset: u64,
    pub content: String,
    pub encoding: String,
    #[allow(dead_code)]
    pub truncated: bool,
}

/// Observed progress for one original transfer. `observed` counts only bytes
/// that were decoded and retained from a real reply. `total` is `None` unless
/// the captured record supplied an explicit, positive size; no percentage is
/// ever inferred from a chunk count or a `truncated` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferProgress {
    pub observed: u64,
    pub total: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_base64_vectors() {
        assert_eq!(STANDARD.decode("TWE=").unwrap(), b"Ma");
        assert!(STANDARD.decode("TR==").is_err());
        assert!(STANDARD.decode("TQ==\n").is_err());
    }

    #[test]
    fn url_and_identity_policy() {
        assert!(validated_base("https://oyster.example.workers.dev").is_ok());
        assert!(validated_base("https://oyster.example.workers.dev:8443").is_ok());
        assert!(validated_base("http://example.com").is_err());
        assert!(validated_base("http://127.0.0.1:8787/my/path?q=1").is_ok());
        let url = validated_base("http://127.0.0.1:8787/my/path?q=1#f").unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:8787/");
        assert!(validated_base("https://user:pw@oyster.example.workers.dev").is_err());

        assert!(valid_installation("prints"));
        assert!(valid_installation("my-prints-2"));
        assert!(!valid_installation("Prints"));
        assert!(!valid_installation("../prints"));
        assert!(valid_folder("Characters/Mira"));
        assert!(!valid_folder("/Characters"));
        assert!(!valid_folder("Characters/../Mira"));
        assert!(valid_folder("Characters"));
    }

    #[test]
    fn synthetic_list_outputs_payload() {
        let raw = r#"{"items":[{"print":{"printId":"print_abc","computerId":"computer-1","kind":"image","model":"flux","format":"png","bytes":1234,"createdAt":1730000000000},"path":"/prints/print.png"}],"next":null}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        assert!(refusal_code(&value).is_none());
        let page: OutputPage = serde_json::from_value(value).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].print.print_id, "print_abc");
        assert_eq!(page.items[0].path, "/prints/print.png");
        assert!(page.next.is_none());
    }

    #[test]
    fn synthetic_refusal_payload() {
        let raw = r#"{"error":"library_unavailable","message":"Choose a Prints library folder on the selected Computer."}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(refusal_code(&value), Some("library_unavailable"));
        let success: Value = serde_json::from_str(r#"{"models":[]}"#).unwrap();
        assert!(refusal_code(&success).is_none());
    }

    #[test]
    fn synthetic_open_output_payload() {
        let raw = r#"{"printId":"print_abc","path":"/prints/print.png","byteOffset":0,"content":"AAECAw==","encoding":"base64","truncated":true}"#;
        let read: ByteRead = serde_json::from_value(serde_json::from_str(raw).unwrap()).unwrap();
        assert_eq!(read.print_id, "print_abc");
        assert_eq!(read.byte_offset, 0);
        assert_eq!(STANDARD.decode(&read.content).unwrap(), vec![0, 1, 2, 3]);
        // The empty content slice, not `truncated`, ends the download.
        let empty = r#"{"printId":"print_abc","path":"/prints/print.png","byteOffset":4,"content":"","encoding":"base64","truncated":false}"#;
        let last: ByteRead = serde_json::from_value(serde_json::from_str(empty).unwrap()).unwrap();
        assert!(last.content.is_empty() && !last.truncated);
    }

    #[test]
    fn typed_http_round_trip_reads_and_reassembles_original_bytes() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let fixture_bytes = b"fictional print original";
        let key = "oyk_local_fixture_123456";
        let server = thread::spawn(move || {
            let reply = |offset, content: String| {
                json!({
                    "printId": "print_fixture", "path": "/prints/fixture.jpg", "byteOffset": offset,
                    "content": content, "encoding": "base64", "truncated": !content.is_empty(),
                })
            };
            let requests = [
                (
                    "listModels",
                    None,
                    json!({"models":[{"id":"fictional-model","label":"Fixture"}]}),
                ),
                ("listOutputs", None, json!({"items":[], "next":null})),
                (
                    "openOutput",
                    Some(0),
                    reply(0, STANDARD.encode(fixture_bytes)),
                ),
                (
                    "openOutput",
                    Some(fixture_bytes.len()),
                    reply(fixture_bytes.len(), String::new()),
                ),
            ];
            for (method, offset, result) in requests {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = BufReader::new(&mut socket);
                let mut line = String::new();
                request.read_line(&mut line).unwrap();
                assert_eq!(
                    line,
                    format!("POST /apps/prints/methods/{method} HTTP/1.1\r\n")
                );
                let mut length = 0;
                let mut matching_auth = false;
                loop {
                    line.clear();
                    request.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.trim().split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                        if name.eq_ignore_ascii_case("authorization") {
                            matching_auth = value.trim() == format!("Bearer {key}");
                        }
                    }
                }
                assert!(matching_auth);
                let mut body = vec![0; length];
                request.read_exact(&mut body).unwrap();
                let input: Value = serde_json::from_slice(&body).unwrap();
                if let Some(offset) = offset {
                    assert_eq!(input["byteOffset"], offset);
                    assert_eq!(input["printId"], "print_fixture");
                }
                let payload = serde_json::to_vec(&result).unwrap();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len()).unwrap();
                socket.write_all(&payload).unwrap();
            }
        });
        let client = OysterClient::new(&origin, "prints", ApiKey::new(key)).unwrap();
        assert_eq!(
            client.test_connection().unwrap().models[0].id,
            "fictional-model"
        );
        assert!(
            client
                .list_outputs(None, true, None, 24)
                .unwrap()
                .items
                .is_empty()
        );
        assert_eq!(
            client.download_output("print_fixture").unwrap(),
            fixture_bytes
        );
        server.join().unwrap();
    }

    #[test]
    fn arbitrary_error_text_is_redacted() {
        assert_eq!(
            refusal_code(&json!({"error": "oyk_echoed_secret"})),
            Some("unknown_refusal")
        );
        assert!(matches!(
            OysterClient::new(
                "https://example.invalid",
                "prints",
                ApiKey::new("owner_key")
            ),
            Err(Error::InvalidKey),
        ));
    }

    #[test]
    fn secret_never_appears_in_debug() {
        let key = ApiKey::new("oyk_super_secret");
        let text = format!("{key:?} {key}");
        assert!(!text.contains("oyk_super_secret"));
        assert!(text.contains("REDACTED"));
    }

    #[test]
    fn diagnostic_keeps_only_bounded_allowlisted_fields() {
        let error = Error::Status {
            status: 502,
            code: Some("oyk_injected_secret".into()),
        };
        let diagnostic = error.diagnostic("discover models", "listModels", 42, None);
        assert_eq!(diagnostic.category, Category::Status);
        assert_eq!(diagnostic.status, Some(502));
        assert_eq!(diagnostic.code, None);
        assert_eq!(diagnostic.elapsed_ms, 42);
        let text = format!("{diagnostic:?}");
        assert!(!text.contains("oyk_injected_secret"));

        let refused = Error::Refused {
            method: "listModels",
            code: "oyk_fixture_key".into(),
        };
        assert_eq!(
            refused
                .diagnostic("discover models", "listModels", 1, None)
                .code,
            None
        );
        let allowed = Error::Refused {
            method: "listModels",
            code: "not_granted".into(),
        };
        assert_eq!(
            allowed
                .diagnostic("discover models", "listModels", 1, None)
                .code
                .as_deref(),
            Some("not_granted")
        );
    }

    #[test]
    fn success_diagnostic_records_observed_fallback() {
        let diagnostic = Diagnostic::success(
            "connect",
            "listOutputs",
            17,
            Some(Fallback::RecursiveReadUnavailable),
        );
        assert_eq!(diagnostic.category, Category::Success);
        assert_eq!(
            diagnostic.fallback.map(Fallback::note),
            Some("root read attempted after recursive read failed")
        );
    }
}
