//! Native connection and library screens. Owner media is held in memory for this session.
use std::path::PathBuf;
use std::sync::Arc;

use prints_desktop::oyster::{
    ApiKey, Cursor as PageCursor, Error, FolderPage, Model, OutputItem, OutputPage, OysterClient,
};
use rune::Latest;

use super::*;

const KEY_SERVICE: &str = "com.loftwah.prints-desktop.oyster";
const PROFILE: &str = "oyster-profile";
const PREVIEW_BYTES: u64 = 8 * 1024 * 1024;
const EXPORT_BYTES: u64 = 32 * 1024 * 1024;
type Profile = (String, String);

struct Connection {
    client: Arc<OysterClient>,
    models: Result<Vec<Model>, Error>,
    page: OutputPage,
    stored: Result<bool, String>,
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

struct Library {
    client: Arc<OysterClient>,
    models: Result<Vec<Model>, Error>,
    position: PagePosition,
    folders: Option<FolderPage>,
    page: OutputPage,
    selected: Option<usize>,
    preview: Option<Image>,
    last_export: Option<PathBuf>,
}

impl Library {
    fn new(client: Arc<OysterClient>, models: Result<Vec<Model>, Error>, page: OutputPage) -> Self {
        let selected = (!page.items.is_empty()).then_some(0);
        Self {
            client,
            models,
            position: PagePosition::first(None),
            folders: None,
            page,
            selected,
            preview: None,
            last_export: None,
        }
    }

    fn show(&mut self, page: OutputPage, position: PagePosition) {
        self.folders = None;
        self.selected = (!page.items.is_empty()).then_some(0);
        self.preview = None;
        self.page = page;
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

impl Transfer {
    fn limit(self) -> u64 {
        match self {
            Self::Preview => PREVIEW_BYTES,
            Self::Original => EXPORT_BYTES,
        }
    }
}

enum Operation {
    Connect,
    Forget,
    LoadPage(PagePosition),
    BrowseFolders,
    Transfer(Transfer),
    ChooseDestination,
    WriteOriginal(OriginalWrite),
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
            Self::ChooseDestination => "Choose where to save the original…",
            Self::WriteOriginal(_) => "Writing original…",
        }
    }
}

enum Reply {
    Connected(Connection),
    Page(OutputPage),
    Folders(FolderPage),
    Preview(Vec<u8>),
    Export(Vec<u8>, String),
    Forgotten,
}

struct Failure {
    message: String,
    unauthorized: bool,
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        let unauthorized = error.authority_lost();
        let message = match &error {
            Error::Status { status: 401, .. } => {
                "The key was refused or revoked. Check your scoped App key.".into()
            }
            Error::Refused { code, .. } | Error::Status { code: Some(code), .. }
                if code == "library_unavailable" =>
            {
                "Choose a gallery folder in web Prints on the selected Oyster Computer, then reconnect."
                    .into()
            }
            Error::Status { status: 403, .. } => {
                "Oyster refused this action. Check the installation grants and selected Computer."
                    .into()
            }
            Error::Status { status: 404, .. } => {
                "The installation or operation is unavailable on this host.".into()
            }
            Error::Transport => {
                "Couldn't reach Oyster. Check the host and your connection, then try again.".into()
            }
            Error::TooLarge => "This file exceeds the transfer size or request limit.".into(),
            _ => error.to_string(),
        };
        Self {
            message,
            unauthorized,
        }
    }
}

impl Failure {
    fn local(message: &str) -> Self {
        Self {
            message: message.into(),
            unauthorized: false,
        }
    }
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
    epoch: u64,
    request: Latest<Result<Reply, Failure>>,
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
            request: Latest::default(),
        }
    }

    pub(super) fn set_waker(&mut self, wake: Waker) {
        self.request.set_waker(wake);
    }

    pub(super) fn connected(&self) -> bool {
        self.library.is_some()
    }

    fn busy(&self) -> bool {
        self.operation.is_some()
    }

    fn start(&mut self, operation: Operation) {
        self.operation = Some(operation);
        self.status.clear();
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
        self.start(Operation::Connect);
        self.request
            .request((profile, key), move |((origin, installation), entered)| {
                let key = if saved {
                    credential(&(origin.clone(), installation.clone()))?
                        .get_password()
                        .map_err(|_| {
                            Failure::local(
                                "The saved key is unavailable. Enter a current key to connect.",
                            )
                        })?
                } else {
                    entered
                };
                let client = Arc::new(OysterClient::new(
                    &origin,
                    &installation,
                    ApiKey::new(key.clone()),
                )?);
                let page = client.list_outputs(None, true, None, 24)?;
                let models = client.list_models().map(|catalogue| catalogue.models);
                let models = match models {
                    Err(error) if error.authority_lost() => return Err(error.into()),
                    other => other,
                };
                let stored = if remember && saved_profile.as_ref().is_some_and(|old| old != &client.identity()) {
                    Err("Connected for this session. Forget the previous saved key before saving a different connection.".into())
                } else if remember {
                    credential(&client.identity())
                        .and_then(|entry| {
                            entry.set_password(&key).map_err(|_| {
                                Failure::local("The OS credential store couldn't save this key.")
                            })
                        })
                        .map(|_| true)
                        .map_err(|error| error.message)
                } else {
                    Ok(false)
                };
                Ok(Reply::Connected(Connection {
                    client,
                    models,
                    page,
                    stored,
                }))
            });
    }

    fn disconnect(&mut self) {
        self.request.cancel();
        self.library = None;
        self.folder = TextEdit::new("");
        self.key = TextEdit::new("").secure();
        self.operation = None;
        self.status.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(super) fn finish_write(&mut self) {
        if matches!(self.operation, Some(Operation::WriteOriginal(_))) {
            self.operation = None;
        }
    }

    fn forget(&mut self) {
        if self.busy() || self.connected() {
            return;
        }
        let Some(profile) = self.saved.clone() else {
            return;
        };
        self.start(Operation::Forget);
        self.request.request(profile, |profile| {
            match credential(&profile)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(Reply::Forgotten),
                Err(_) => Err(Failure::local(
                    "The OS credential store couldn't remove the key.",
                )),
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
        self.start(Operation::LoadPage(position));
        self.request
            .request((client, folder, cursor), |(client, folder, cursor)| {
                Ok(Reply::Page(client.list_outputs(
                    folder.as_deref(),
                    true,
                    cursor,
                    24,
                )?))
            });
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
        self.start(Operation::BrowseFolders);
        self.request
            .request((client, folder), move |(client, folder)| {
                Ok(Reply::Folders(
                    client.library_folders(folder.as_deref(), after)?,
                ))
            });
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
        if self.busy() {
            return;
        }
        let Some(library) = &self.library else {
            return;
        };
        let Some(item) = library.selected().cloned() else {
            return;
        };
        let limit = transfer.limit();
        if item.print.bytes > limit {
            self.status = format!(
                "This original exceeds the {} MiB limit.",
                limit / (1024 * 1024)
            );
            return;
        }
        let client = library.client.clone();
        self.start(Operation::Transfer(transfer));
        self.request.request((client, item), move |(client, item)| {
            let bytes = client.verify_and_download(&item)?;
            match transfer {
                Transfer::Preview if !previewable(&bytes) => Err(Failure::local(
                    "This image format can be exported or opened in web Prints.",
                )),
                Transfer::Preview => Ok(Reply::Preview(bytes)),
                Transfer::Original => Ok(Reply::Export(bytes, filename(&item.path))),
            }
        });
    }
}

fn credential(profile: &Profile) -> Result<keyring::Entry, Failure> {
    let (origin, installation) = profile;
    keyring::Entry::new(KEY_SERVICE, &format!("{origin}/{installation}"))
        .map_err(|_| Failure::local("Your OS credential store is unavailable."))
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
    let h = &mut s.hosted;
    if let Some(Operation::WriteOriginal(write)) = &h.operation {
        let Some(result) = write.take() else {
            return false;
        };
        h.operation = None;
        match result {
            Ok(path) => {
                if let Some(library) = &mut h.library {
                    library.last_export = Some(path);
                }
                h.status = "Original exported.".into();
            }
            Err(_) => h.status = "Couldn't write the selected file.".into(),
        }
        return true;
    }
    let Some(reply) = h.request.take() else {
        return false;
    };
    let Some(operation) = h.operation.take() else {
        return false;
    };
    match (operation, reply) {
        (Operation::Connect, Ok(Reply::Connected(connection))) => {
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
            h.library = Some(Library::new(
                connection.client,
                connection.models,
                connection.page,
            ));
            if s.screen == Screen::Connection {
                s.screen = Screen::Hosted;
                tx.blur();
            }
        }
        (Operation::LoadPage(position), Ok(Reply::Page(page))) => {
            if let Some(library) = &mut h.library {
                library.show(page, position);
            }
        }
        (Operation::BrowseFolders, Ok(Reply::Folders(folders))) => {
            if let Some(library) = &mut h.library {
                library.folders = Some(folders);
            }
        }
        (Operation::Transfer(Transfer::Preview), Ok(Reply::Preview(bytes))) => {
            if let (Some(library), Some(cache)) = (&mut h.library, &s.cache) {
                library.preview = Some(cache.load(Arc::<[u8]>::from(bytes)));
            }
        }
        (Operation::Transfer(Transfer::Original), Ok(Reply::Export(bytes, name))) => {
            h.start(Operation::ChooseDestination);
            let epoch = h.epoch;
            tx.save_file(
                FileDialogOptions {
                    title: "Export Prints original".into(),
                    file_name: Some(name),
                    ..FileDialogOptions::default()
                },
                move |s: &mut Studio, result, _| {
                    let h = &mut s.hosted;
                    if h.epoch != epoch
                        || !matches!(h.operation, Some(Operation::ChooseDestination))
                    {
                        return;
                    }
                    match result {
                        Ok(Some(path)) => {
                            match OriginalWrite::start(path, bytes.into(), s.wake.clone()) {
                                Ok(write) => h.start(Operation::WriteOriginal(write)),
                                Err(_) => {
                                    h.operation = None;
                                    h.status = "Couldn't write the selected file.".into();
                                }
                            }
                        }
                        Ok(None) => {
                            h.operation = None;
                            h.status = "Export cancelled.".into();
                        }
                        Err(_) => {
                            h.operation = None;
                            h.status = "Couldn't open the Save dialog.".into();
                        }
                    }
                },
            );
        }
        (Operation::Forget, Ok(Reply::Forgotten)) => {
            h.saved = None;
            h.status = if s.prefs.remove(PROFILE).is_ok() {
                "Saved key removed from this device.".into()
            } else {
                "The key was removed. The app couldn't clear the saved connection label.".into()
            };
        }
        (operation, Err(error)) => {
            if error.unauthorized && h.connected() {
                h.disconnect();
                if s.screen == Screen::Hosted {
                    s.screen = Screen::Connection;
                    tx.blur();
                }
            }
            h.status = error.message;
            if matches!(operation, Operation::LoadPage(_)) && h.connected() {
                h.status.push_str(" Showing the previous view.");
            }
        }
        _ => h.status = "The library operation did not finish as expected. Try again.".into(),
    }
    true
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
        if item.print.bytes <= EXPORT_BYTES {
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
    match &library.models {
        Ok(models) => {
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
        Err(_) => {
            catalogue = catalogue.child(
                label(
                    "Model catalogue unavailable for this connection.",
                    13.0,
                    500.0,
                    p.muted,
                )
                .wrap(),
            )
        }
    }
    detail = detail.child(catalogue);
    let mut feedback = row().gap(10.0).items_center().child(status(h, p));
    if let Some(path) = &library.last_export {
        let path = path.clone();
        feedback = feedback.child(action(
            "hosted-reveal",
            "Show last export",
            p,
            false,
            move |s, tx| {
                if tx.reveal_path(&path).is_err() {
                    s.hosted.status = "Couldn't show that file in the desktop file manager.".into();
                }
            },
        ));
    }
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
