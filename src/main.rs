#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod agent_bridge;
mod archive;
mod comparison;
mod disk;
use disk::OriginalWrite;
mod draft;
mod hosted;
mod local;
mod local_images;
mod navigation;
mod tooling_mode;
mod viewer;
mod workspace;

use workspace::{AccessMode, AgentAccess, AssetId, Fault, Surface, WorkspaceId};

use rune::prefs::Prefs;
use rune::tok::TextStyle;
use rune::{
    App, AppSpec, Color, Cursor, Cx, El, FileDialogOptions, Image, ImageCache, Key, KeyEvent,
    Latest, ScrollSpec, SurfaceFit, TextEdit, Tx, Waker, col, el, row, text, text_input,
};

const APP_ID: &str = "com.loftwah.prints-desktop";
const MAX_IMPORT: u64 = 24 * 1024 * 1024;
const CONTACT_PAGE_SIZE: usize = 12;

struct Study {
    title: &'static str,
    note: &'static str,
    prompt: &'static str,
    filename: &'static str,
    bytes: &'static [u8],
}

const AFTERLIGHT: [Study; 6] = [
    Study {
        title: "The Last Horizon",
        note: "A setting sun held inside a concrete ring on a volcanic coast.",
        prompt: "Frame a vermilion sun inside a monumental ivory concrete ring. Place it on a volcanic coastline, with still water, layered mist and ink-plum shadows. Use the colour and grain of a large-format film photograph.",
        filename: "the-last-horizon.jpg",
        bytes: include_bytes!("../assets/horizon.jpg"),
    },
    Study {
        title: "Relay Station",
        note: "An antenna tower holds its ground as the dunes move around it.",
        prompt: "Photograph a solitary rust-red relay tower through blowing copper sand. Keep the tower small against the dunes and use the last side light to bring out the ridges.",
        filename: "relay-station.jpg",
        bytes: include_bytes!("../assets/relay.jpg"),
    },
    Study {
        title: "Salt Memory",
        note: "One dark rock interrupts a plane of fractured pink salt.",
        prompt: "Look down at a shallow angle over a salmon-pink salt pan. Isolate a single black rock against the cracked crust and let the texture of the salt fill the frame.",
        filename: "salt-memory.jpg",
        bytes: include_bytes!("../assets/salt.jpg"),
    },
    Study {
        title: "Blue Hour",
        note: "A lavender beacon marks a way through the frozen pass.",
        prompt: "Place a distant lavender beacon in an icy cobalt mountain pass at twilight. Use layered fog, dark foreground rocks and a single point of light to reveal the depth of the valley.",
        filename: "blue-hour.jpg",
        bytes: include_bytes!("../assets/blue-hour.jpg"),
    },
    Study {
        title: "Airlock Garden",
        note: "Ferns and moss take back an abandoned airlock.",
        prompt: "Photograph the interior of an abandoned concrete airlock. A shaft of daylight finds silver ferns and wet moss around the circular door. Keep the shadows deep and the architecture tactile.",
        filename: "airlock-garden.jpg",
        bytes: include_bytes!("../assets/airlock.jpg"),
    },
    Study {
        title: "Echo Chamber",
        note: "Floodwater holds the reflection of an empty concrete enclosure.",
        prompt: "Set tall concrete slabs in a flooded plain. Use staggered repetition, dark purple water and a distant magenta flash to give the landscape a sense of scale.",
        filename: "echo-chamber.jpg",
        bytes: include_bytes!("../assets/echo.jpg"),
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shelf {
    Archive,
    Samples,
    Favourites,
    Imports,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Workspace,
    Preview,
    Compare(comparison::Pair),
    Draft,
    Connection,
    Hosted,
    Agent,
}

struct Picture {
    bytes: Arc<[u8]>,
    image: Image,
}

impl Picture {
    fn new(bytes: Arc<[u8]>, cache: &ImageCache) -> Self {
        Self {
            image: cache.load(bytes.clone()),
            bytes,
        }
    }
}

enum Original {
    Afterlight(&'static Study, Picture),
    Edition(&'static archive::Edition, Picture),
    Linked { path: PathBuf, available: bool },
}

struct Print {
    id: AssetId,
    title: String,
    filename: String,
    find: String,
    original: Original,
}

impl Print {
    fn new(title: String, filename: String, original: Original, id: AssetId) -> Self {
        let mut fields = vec![title.as_str(), filename.as_str()];
        match &original {
            Original::Afterlight(study, _) => fields.extend([study.note, study.prompt]),
            Original::Edition(edition, _) => fields.extend([edition.note, edition.source]),
            Original::Linked { .. } => (),
        }
        let find = fields.join("\n").to_lowercase();
        Self {
            id,
            title,
            filename,
            find,
            original,
        }
    }

    fn linked(path: PathBuf, filename: Option<String>, id: AssetId) -> Self {
        let title = path.file_stem().map_or_else(
            || "Saved local image".into(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let filename = filename.unwrap_or_else(|| {
            path.file_name()
                .map_or_else(|| title.clone(), |name| name.to_string_lossy().into_owned())
        });
        Self::new(
            title,
            filename,
            Original::Linked {
                path,
                available: true,
            },
            id,
        )
    }

    fn id(&self) -> &AssetId {
        &self.id
    }

    fn study(&self) -> Option<&'static Study> {
        match self.original {
            Original::Afterlight(study, _) => Some(study),
            _ => None,
        }
    }

    fn edition(&self) -> Option<&'static archive::Edition> {
        match self.original {
            Original::Edition(edition, _) => Some(edition),
            _ => None,
        }
    }

    fn path(&self) -> Option<&Path> {
        match &self.original {
            Original::Linked { path, .. } => Some(path),
            _ => None,
        }
    }

    fn picture(&self) -> Option<&Picture> {
        match &self.original {
            Original::Afterlight(_, picture) | Original::Edition(_, picture) => Some(picture),
            Original::Linked { .. } => None,
        }
    }

    fn available(&self) -> bool {
        !matches!(
            self.original,
            Original::Linked {
                available: false,
                ..
            }
        )
    }

    fn caption(&self) -> &'static str {
        match &self.original {
            Original::Afterlight(study, _) => study.note,
            Original::Edition(edition, _) => edition.source,
            Original::Linked {
                available: true, ..
            } => "Linked original on this device.",
            Original::Linked {
                available: false, ..
            } => "Original unavailable — refresh or locate it.",
        }
    }
}

#[derive(Default)]
struct Search {
    input: TextEdit,
    words: Vec<String>,
}

impl Search {
    fn update(&mut self) {
        self.words = self
            .input
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
    }

    fn clear(&mut self) {
        self.input.clear();
        self.words.clear();
    }

    fn includes(&self, normalized: &str) -> bool {
        self.words.iter().all(|word| normalized.contains(word))
    }
}

#[derive(Default)]
enum LocalExport {
    #[default]
    Idle,
    ChooseDestination,
    WriteOriginal(OriginalWrite),
}

struct Studio {
    shelf: Shelf,
    screen: Screen,
    assets: Vec<Print>,
    selected: Option<usize>,
    comparison_first: Option<AssetId>,
    human_edits: u64,
    ids_migrated: bool,
    agent: AgentAccess,
    bridge: agent_bridge::Bridge,
    agent_focus: Option<String>,
    agent_save: bool,
    favourites: Vec<usize>,
    theme_light: bool,
    inspector: bool,
    draft: TextEdit,
    working: draft::Store,
    draft_shortlist: Vec<usize>,
    search: Search,
    status: String,
    prefs: Prefs,
    cache: Option<Arc<ImageCache>>,
    local_images: local_images::LocalImages,
    import_request: Latest<ImportBatch>,
    import_origin: Option<(Screen, Shelf)>,
    local_export: LocalExport,
    wake: Option<Waker>,
    local_loaded: bool,
    proof: bool,
    hosted: hosted::Hosted,
    viewer: viewer::Viewer,
    preview_return: Option<String>,
}

impl Studio {
    fn new() -> Self {
        let proof = tooling_mode::isolated();
        let prefs = tooling_mode::preferences(APP_ID);
        let mut favourites: Vec<usize> = prefs.get("favourites").unwrap_or_default();
        favourites.retain(|&i| i < AFTERLIGHT.len());
        favourites.sort_unstable();
        favourites.dedup();
        let archive_favourites = prefs
            .get::<Vec<String>>("archive-favourites")
            .unwrap_or_default();
        favourites.extend(
            archive::EDITIONS
                .iter()
                .enumerate()
                .filter_map(|(i, edition)| {
                    archive_favourites
                        .iter()
                        .any(|name| name == edition.filename)
                        .then_some(AFTERLIGHT.len() + i)
                }),
        );
        let theme_light = prefs.get("light").unwrap_or(false);
        let working = draft::Store::open(&prefs);
        let text = working.loaded.as_ref().map_or_else(
            || prefs.get::<String>("draft").unwrap_or_default(),
            |saved| saved.text.clone(),
        );
        let draft = TextEdit::new(text);
        let hosted = hosted::Hosted::new(&prefs);
        Self {
            shelf: Shelf::Archive,
            screen: Screen::Workspace,
            assets: Vec::new(),
            selected: Some(AFTERLIGHT.len()),
            comparison_first: None,
            human_edits: 0,
            ids_migrated: false,
            agent: AgentAccess::off(working.workspace_id.clone()),
            bridge: agent_bridge::Bridge::default(),
            agent_focus: None,
            agent_save: false,
            favourites,
            theme_light,
            inspector: true,
            draft,
            working,
            draft_shortlist: Vec::new(),
            search: Search::default(),
            status: String::new(),
            prefs,
            cache: None,
            local_images: local_images::LocalImages::default(),
            import_request: Latest::default(),
            import_origin: None,
            local_export: LocalExport::Idle,
            wake: None,
            local_loaded: false,
            proof,
            hosted,
            viewer: viewer::Viewer::default(),
            preview_return: None,
        }
    }

    fn load(&mut self, wake: Waker) {
        self.hosted.set_waker(wake.clone());
        self.import_request.set_waker(wake.clone());
        self.local_images.set_waker(wake.clone());
        self.wake = Some(wake.clone());
        let cache = Arc::new(ImageCache::new(wake));
        self.assets = AFTERLIGHT
            .iter()
            .map(|study| {
                Print::new(
                    study.title.into(),
                    study.filename.into(),
                    Original::Afterlight(study, Picture::new(study.bytes.into(), &cache)),
                    AssetId::bundled("afterlight", study.filename),
                )
            })
            .collect();
        self.assets.extend(archive::EDITIONS.iter().map(|edition| {
            Print::new(
                edition.title.into(),
                edition.filename.into(),
                Original::Edition(edition, Picture::new(edition.bytes.into(), &cache)),
                AssetId::bundled("archive", edition.filename),
            )
        }));
        self.ids_migrated |= draft::restore_bundled(self);
        if self.proof {
            for picture in self.assets.iter().filter_map(Print::picture) {
                let _ = picture.image.wait();
            }
        }
        self.cache = Some(cache);
    }

    fn open_local_library(&mut self) {
        if self.local_loaded {
            return;
        }
        self.local_loaded = true;
        let saved = self.working.loaded.take();
        let (records, shortlist) = if let Some(saved) = saved {
            (Some(saved.library), Some(saved.shortlist))
        } else {
            (self.prefs.get(local::LIBRARY_KEY), None)
        };
        let mut records = local::read(records);
        if local::migrate_ids(&mut records) {
            self.ids_migrated = true;
        }
        for record in records {
            if record.favourite {
                self.favourites.push(self.assets.len());
            }
            if shortlist.is_none() && record.in_draft {
                self.draft_shortlist.push(self.assets.len());
            }
            self.assets
                .push(Print::linked(record.path, None, record.id));
        }
        if let Some(entries) = shortlist {
            self.draft_shortlist = draft::resolve(&self.assets, &entries);
        }
    }

    fn original<'a>(&'a self, asset: &'a Print) -> Option<&'a Picture> {
        asset.picture().or_else(|| {
            self.local_images
                .original(asset.path()?)
                .map(|image| &image.picture)
        })
    }

    fn poll_local_images(&mut self) -> bool {
        let Some(cache) = &self.cache else {
            return false;
        };
        let originals = match self.screen {
            Screen::Workspace | Screen::Preview => [self.selected, None],
            Screen::Compare(comparison::Pair::Local(first, second)) => [Some(second), Some(first)],
            _ => [None; 2],
        };
        let path_at = |index| {
            self.assets
                .get(index)
                .and_then(Print::path)
                .map(Path::to_path_buf)
        };
        let originals: Vec<_> = originals
            .into_iter()
            .flatten()
            .filter_map(path_at)
            .collect();
        let mut thumbnails = Vec::new();
        if self.screen == Screen::Workspace {
            let visible = self.visible();
            thumbnails.extend(
                visible[contact_page(&visible, self.selected)]
                    .iter()
                    .copied()
                    .filter_map(path_at),
            );
        }
        let ready = self.local_images.poll(&originals, &thumbnails, cache);
        let changed = ready.iter().any(Option::is_some);
        for path in ready.into_iter().flatten() {
            let Some(asset) = self
                .assets
                .iter_mut()
                .find(|item| item.path() == Some(path.as_path()))
            else {
                continue;
            };
            if let Original::Linked { available, .. } = &mut asset.original
                && let Some(loaded) = self.local_images.available(&path)
            {
                *available = loaded;
            }
        }
        changed
    }

    fn refs(&self) -> impl Iterator<Item = &Path> {
        self.assets.iter().filter_map(Print::path)
    }

    fn persist_local(&mut self) -> bool {
        if !draft::persist(self) {
            self.status =
                "The local library could not be saved. Changes apply for this session.".into();
            return false;
        }
        true
    }

    fn visible(&self) -> Vec<usize> {
        self.assets
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let on_shelf = match self.shelf {
                    Shelf::Archive => item.edition().is_some(),
                    Shelf::Samples => item.study().is_some(),
                    Shelf::Favourites => self.favourites.contains(&i),
                    Shelf::Imports => item.path().is_some(),
                };
                (on_shelf && self.search.includes(&item.find)).then_some(i)
            })
            .collect()
    }

    fn searching(&self) -> bool {
        !self.search.words.is_empty()
    }

    fn sync_selection(&mut self) {
        let visible = self.visible();
        if self.selected.is_none_or(|i| !visible.contains(&i)) {
            self.selected = visible.first().copied();
            self.touch();
        }
    }

    fn search_changed(&mut self, tx: &mut Tx) {
        self.search.update();
        self.sync_selection();
        self.status.clear();
        self.reveal_selection(tx);
    }

    fn focus_search(&mut self, tx: &mut Tx) {
        self.screen = Screen::Workspace;
        self.search
            .input
            .set_selection(0, self.search.input.text.len());
        tx.focus("search-input", true);
    }

    fn clear_search(&mut self) {
        self.search.clear();
        self.sync_selection();
        self.status.clear();
    }

    fn show_shelf(&mut self, shelf: Shelf) {
        if matches!(shelf, Shelf::Imports | Shelf::Favourites) {
            self.open_local_library();
        }
        self.shelf = shelf;
        self.screen = Screen::Workspace;
        self.status.clear();
        self.search.clear();
        self.sync_selection();
    }

    fn reveal_selection(&self, tx: &mut Tx) {
        if let Some(index) = self.selected {
            tx.scroll_into_view_in("contact-sheet", format!("print-{index}"));
        }
    }

    /// Enter full view, remembering the control that initiated it so the return can restore that
    /// focus instead of dropping it.
    fn open_preview(&mut self, tx: &mut Tx, return_to: &'static str) {
        if self.screen != Screen::Preview {
            self.preview_return = Some(return_to.into());
            self.viewer.reset();
        }
        self.screen = Screen::Preview;
        tx.focus("close-preview", tx.by_keys() || tx.focus_visible());
    }

    /// Return from full view with the initiating focus restored.
    fn close_preview(&mut self, tx: &mut Tx) {
        self.screen = Screen::Workspace;
        if let Some(key) = self.preview_return.take() {
            tx.focus(key, tx.by_keys() || tx.focus_visible());
        } else {
            tx.blur();
        }
    }

    fn step(&mut self, direction: isize) {
        let list = self.visible();
        if list.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|i| list.iter().position(|&x| x == i))
            .unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(list.len() as isize) as usize;
        self.selected = Some(list[next]);
        self.status.clear();
        self.touch();
        if self.screen == Screen::Preview {
            self.viewer.reset();
        }
    }

    fn turn_contact_page(&mut self, forward: bool, tx: &mut Tx) {
        let visible = self.visible();
        let current = contact_page(&visible, self.selected);
        let position = if forward {
            current.end
        } else {
            current.start.saturating_sub(CONTACT_PAGE_SIZE)
        };
        if let Some(&index) = visible.get(position) {
            self.selected = Some(index);
            self.status.clear();
            tx.scroll_to_instant("contact-sheet", 0.0, 0.0);
        }
    }

    fn favourite(&mut self) {
        let Some(i) = self.selected else { return };
        self.touch();
        if self.favourites.contains(&i) {
            self.favourites.retain(|&value| value != i);
        } else {
            self.favourites.push(i);
        }
        if i < AFTERLIGHT.len() {
            let samples: Vec<usize> = self
                .favourites
                .iter()
                .copied()
                .filter(|&i| i < AFTERLIGHT.len())
                .collect();
            self.status = if self.prefs.set("favourites", &samples).is_ok() {
                String::new()
            } else {
                "Favourites could not be saved to disk.".into()
            };
        } else if self.assets[i].edition().is_some() {
            let archive_favourites: Vec<&str> = self
                .favourites
                .iter()
                .filter_map(|&i| self.assets[i].edition().map(|edition| edition.filename))
                .collect();
            self.status = if self
                .prefs
                .set("archive-favourites", &archive_favourites)
                .is_ok()
            {
                String::new()
            } else {
                "Favourites could not be saved to disk.".into()
            };
        } else if self.persist_local() {
            self.status.clear();
        }
        if self.shelf == Shelf::Favourites && !self.favourites.contains(&i) {
            self.selected = self.visible().first().copied();
            if self.selected.is_none() {
                self.screen = Screen::Workspace;
            }
        }
    }

    fn toggle_theme(&mut self) {
        self.theme_light = !self.theme_light;
        if self.prefs.set("light", &self.theme_light).is_err() {
            self.status = "The theme could not be saved to disk.".into();
        }
    }

    fn import(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if self.import_origin.is_some() {
            self.status = "Wait for the current import to finish.".into();
            return;
        }
        self.open_local_library();
        if self.cache.is_none() {
            self.status = "The library is starting. Try again in a moment.".into();
            return;
        }
        let existing: Vec<_> = self.refs().map(Path::to_path_buf).collect();
        self.import_origin = Some((self.screen, self.shelf));
        self.status = "Linking images…".into();
        self.import_request
            .request((paths, existing), |(paths, existing)| {
                ImportBatch::read(paths, existing)
            });
    }

    fn finish_import(&mut self, tx: &mut Tx) -> bool {
        let Some(batch) = self.import_request.take() else {
            return false;
        };
        let origin = self.import_origin.take();
        let start = self.assets.len();
        let mut used: HashSet<String> = self
            .assets
            .iter()
            .map(|asset| asset.id().as_str().to_owned())
            .collect();
        for (path, original) in batch.images {
            // A linked image can be relocated while the batch is being read.
            if !self.refs().any(|existing| existing == path) {
                let id = AssetId::fresh(&used);
                used.insert(id.as_str().to_owned());
                self.assets.push(Print::linked(path, Some(original), id));
            }
        }
        let linked = self.assets.len() - start;
        if linked > 0 {
            self.touch();
            if origin == Some((self.screen, self.shelf)) {
                self.show_shelf(Shelf::Imports);
                self.selected = Some(start);
                tx.blur();
                self.reveal_selection(tx);
            } else if self.screen == Screen::Workspace && self.shelf == Shelf::Imports {
                self.sync_selection();
            }
            if !self.persist_local() {
                return true;
            }
        }
        self.status = match (linked, batch.total) {
            (0, 1) => batch.first_error.unwrap_or(DUPLICATE_IMPORT).into(),
            (1, 1) => "Image linked.".into(),
            (linked, total) if linked == total => format!("Linked {linked} images."),
            (linked, total) if batch.full => {
                format!(
                    "Linked {linked} of {total} files. Library limit: {}.",
                    local::MAX_REFS
                )
            }
            (linked, total) => format!(
                "Linked {linked} of {total} files. Skipped duplicates or unreadable images."
            ),
        };
        true
    }

    /// Re-read the stored path, so an original that returns is recovered.
    fn refresh(&mut self, tx: &mut Tx) {
        let Some(index) = self.selected else { return };
        let Some(path) = self.assets.get(index).and_then(Print::path) else {
            return;
        };
        match validate_original(path) {
            Ok(filename) => {
                let id = self.assets[index].id().clone();
                self.local_images.invalidate(path);
                self.assets[index] = Print::linked(path.to_path_buf(), Some(filename), id);
                self.sync_selection();
                self.focus_after_recovery(index, tx);
                self.status = "Original refreshed.".into();
            }
            Err(err) => self.status = err.restore_message().into(),
        }
    }

    /// Point this reference at a new canonical path chosen by the user.
    fn relocate(&mut self, previous: &Path, path: &Path, tx: &mut Tx) {
        let Some(index) = self
            .assets
            .iter()
            .position(|asset| asset.path() == Some(previous))
        else {
            self.status = "That entry is no longer in the local library.".into();
            return;
        };
        let source = match local::canonical(path) {
            Some(source) => source,
            None => {
                self.status = "That file could not be read.".into();
                return;
            }
        };
        let duplicate = self
            .assets
            .iter()
            .enumerate()
            .any(|(i, asset)| i != index && asset.path() == Some(source.as_path()));
        if duplicate {
            self.status = "That image is already in your local library.".into();
            return;
        }
        match validate_original(&source) {
            Ok(filename) => {
                let id = self.assets[index].id().clone();
                self.local_images.invalidate(previous);
                self.touch();
                self.assets[index] = Print::linked(source, Some(filename), id);
                self.sync_selection();
                self.focus_after_recovery(index, tx);
                if self.persist_local() {
                    self.status = "Original located.".into();
                }
            }
            Err(err) => self.status = err.restore_message().into(),
        }
    }

    fn focus_after_recovery(&self, index: usize, tx: &mut Tx) {
        tx.focus(
            if self.selected == Some(index) {
                "featured-image"
            } else {
                "search-input"
            },
            true,
        );
    }

    /// Drop only this reference and its favourite; the original file is untouched.
    fn remove_local(&mut self) {
        let Some(index) = self.selected else { return };
        let Some(path) = self.assets.get(index).and_then(Print::path) else {
            return;
        };
        let id = self.assets.get(index).map(Print::id).cloned();
        if self.comparison_first == id && id.is_some() {
            self.comparison_first = None;
        }
        self.local_images.invalidate(path);
        self.touch();
        self.assets.remove(index);
        let removed_from_draft = draft::forget_asset(self, index);
        self.favourites.retain(|&i| i != index);
        for favourite in &mut self.favourites {
            if *favourite > index {
                *favourite -= 1;
            }
        }
        self.sync_selection();
        if self.persist_local() {
            self.status = if removed_from_draft {
                "Removed from the library and draft. Your original file is unchanged.".into()
            } else {
                "Removed from library. Your original file is unchanged.".into()
            };
        }
    }

    fn use_prompt(&mut self, tx: &mut Tx) {
        let Some(index) = self.selected else { return };
        let Some(study) = self.assets.get(index).and_then(Print::study) else {
            return;
        };
        if self.draft.text.trim().is_empty() {
            self.draft = TextEdit::new(study.prompt);
            self.touch();
            self.working.pending(tx);
            self.status.clear();
            if !self.draft_shortlist.contains(&index) {
                draft::toggle(self, index);
            }
        } else {
            self.status = "Your draft is still here. Clear it first to use this brief.".into();
        }
        self.open_local_library();
        self.screen = Screen::Draft;
    }

    fn save_draft(&mut self) {
        draft::persist(self);
    }

    /// Bump the human-edit part of the revision. Visible state also feeds the
    /// revision, so a missed touch cannot make a stale write look current.
    fn touch(&mut self) {
        self.human_edits = self.human_edits.wrapping_add(1);
    }

    fn asset_index(&self, id: &AssetId) -> Option<usize> {
        self.assets.iter().position(|asset| asset.id() == id)
    }

    /// Built from live state every time a command is evaluated.
    fn revision(&self) -> u64 {
        let shortlist: Vec<AssetId> = self
            .draft_shortlist
            .iter()
            .filter_map(|&index| self.assets.get(index).map(|asset| asset.id().clone()))
            .collect();
        let assets: Vec<AssetId> = self.assets.iter().map(|asset| asset.id().clone()).collect();
        let selected = self
            .selected
            .and_then(|index| self.assets.get(index))
            .map(Print::id);
        let compare_second = match self.screen {
            Screen::Compare(comparison::Pair::Local(_, second)) => {
                self.assets.get(second).map(Print::id)
            }
            _ => None,
        };
        workspace::revision_of(&workspace::RevisionParts {
            human_edits: self.human_edits,
            brief: &self.draft.text,
            selected,
            shortlist: &shortlist,
            compare_first: self.comparison_first.as_ref(),
            compare_second,
            assets: &assets,
        })
    }

    fn comparison_assets(&self) -> Option<[usize; 2]> {
        match self.screen {
            Screen::Compare(comparison::Pair::Local(first, second)) => Some([first, second]),
            Screen::Compare(comparison::Pair::Campaign) => {
                let index = |name| {
                    self.assets
                        .iter()
                        .position(|asset| asset.edition().is_some() && asset.filename == name)
                };
                let [cover, poster] = comparison::CAMPAIGN_PRINTS;
                Some([index(cover)?, index(poster)?])
            }
            _ => None,
        }
    }

    fn asset_view(&self, index: usize, print: &Print) -> Option<workspace::AssetView> {
        let selected = self.selected == Some(index);
        let in_brief = self.draft_shortlist.contains(&index);
        let active_pair = self.comparison_assets();
        let compare_first = self.comparison_first.as_ref() == Some(print.id())
            || active_pair.is_some_and(|[first, _]| first == index);
        let compare_second = active_pair.is_some_and(|[_, second]| second == index);
        if print.path().is_some() && !(selected || in_brief || compare_first || compare_second) {
            return None;
        }
        Some(workspace::AssetView {
            id: print.id().clone(),
            title: print.title.chars().take(140).collect(),
            selected,
            in_brief,
            comparable: print.available(),
            compare_first,
            compare_second,
        })
    }

    fn show_agent_asset(&mut self, index: usize) {
        let print = &self.assets[index];
        let shelf = if print.path().is_some() {
            Shelf::Imports
        } else if print.study().is_some() {
            Shelf::Samples
        } else {
            Shelf::Archive
        };
        self.show_shelf(shelf);
        self.selected = Some(index);
    }

    // Native hooks shared by the Agent access panel and the transport.
    pub(crate) fn agent_access(&self) -> &AgentAccess {
        &self.agent
    }

    fn agent_activate(&mut self, mode: AccessMode) {
        self.bridge.stop();
        self.agent.revoke();
        let Some(directory) = self
            .prefs
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
        else {
            self.status = "Agent access needs a local workspace profile.".into();
            return;
        };
        let Some(wake) = self.wake.clone() else {
            self.status = "The workspace is still opening. Try again.".into();
            return;
        };
        if !draft::persist(self) {
            self.status = "Couldn’t save this workspace. Save the draft and try again.".into();
            return;
        }
        if self.agent.activate(mode).is_err() {
            self.status = "Couldn’t generate a private access key on this device.".into();
            return;
        }
        let token = self.agent.bearer().expect("access was just enabled");
        if self
            .bridge
            .start(&directory, self.agent.workspace().as_str(), token, wake)
            .is_err()
        {
            self.agent.revoke();
            self.status = "Couldn’t open local agent access. Try again.".into();
            return;
        }
        self.status = "Copy the grant path to use this workspace with your local agent.".into();
    }

    fn agent_revoke(&mut self) {
        self.bridge.stop();
        self.agent.revoke();
        self.status = "Agent access revoked.".into();
    }

    pub(crate) fn open_agent(&mut self) {
        self.status.clear();
        self.screen = Screen::Agent;
    }
}

impl Surface for Studio {
    fn workspace_id(&self) -> &WorkspaceId {
        self.agent.workspace()
    }

    fn access(&self) -> &AgentAccess {
        &self.agent
    }

    fn access_mut(&mut self) -> &mut AgentAccess {
        &mut self.agent
    }

    fn revision(&self) -> u64 {
        Studio::revision(self)
    }

    fn snapshot(&self, limit: usize) -> workspace::BriefSnapshot {
        let mut permitted = self
            .assets
            .iter()
            .enumerate()
            .filter_map(|(index, print)| self.asset_view(index, print));
        let assets = permitted.by_ref().take(limit).collect();
        let mut text = self.draft.text.chars();
        let brief = text.by_ref().take(8_000).collect();
        workspace::BriefSnapshot {
            brief,
            brief_truncated: text.next().is_some(),
            shortlist: self
                .draft_shortlist
                .iter()
                .filter_map(|&index| self.assets.get(index).map(|asset| asset.id().clone()))
                .collect(),
            comparison: self
                .comparison_assets()
                .map(|pair| pair.map(|index| self.assets[index].id().clone())),
            selected: self
                .selected
                .and_then(|index| self.assets.get(index))
                .map(|asset| asset.id().clone()),
            assets,
            has_more: permitted.next().is_some(),
        }
    }

    fn asset(&self, id: &AssetId) -> Option<workspace::AssetView> {
        let index = self.asset_index(id)?;
        self.asset_view(index, self.assets.get(index)?)
    }

    fn select(&mut self, id: &AssetId) -> Result<workspace::Change, Fault> {
        let index = self.asset_index(id).ok_or(Fault::NotFound)?;
        if self.selected == Some(index) && self.screen == Screen::Workspace {
            return Ok(workspace::Change::Unchanged(
                "That image is already selected.",
            ));
        }
        self.show_agent_asset(index);
        self.status.clear();
        self.agent_focus = Some("featured-image".into());
        Ok(workspace::Change::Applied)
    }

    fn compare(&mut self, first: &AssetId, second: &AssetId) -> Result<workspace::Change, Fault> {
        let first_index = self.asset_index(first).ok_or(Fault::NotFound)?;
        let second_index = self.asset_index(second).ok_or(Fault::NotFound)?;
        self.show_agent_asset(second_index);
        self.comparison_first = Some(first.clone());
        comparison::start(self, comparison::Pair::Local(first_index, second_index));
        self.agent_focus = Some("close-comparison".into());
        Ok(workspace::Change::Applied)
    }

    fn edit_brief(&mut self, text: String) -> Result<workspace::Change, Fault> {
        if self.draft.text == text {
            return Ok(workspace::Change::Unchanged("The brief is unchanged."));
        }
        self.draft = TextEdit::new(text);
        self.agent_save = true;
        Ok(workspace::Change::Applied)
    }

    fn keep(&mut self, id: &AssetId) -> Result<workspace::Change, Fault> {
        let index = self.asset_index(id).ok_or(Fault::NotFound)?;
        if self.draft_shortlist.contains(&index) {
            return Ok(workspace::Change::Unchanged("Already kept with the brief."));
        }
        if self.draft_shortlist.len() >= draft::SHORTLIST_LIMIT {
            return Err(Fault::Limit);
        }
        draft::toggle(self, index);
        Ok(workspace::Change::Applied)
    }

    fn remove(&mut self, id: &AssetId) -> Result<workspace::Change, Fault> {
        let index = self.asset_index(id).ok_or(Fault::NotFound)?;
        if !self.draft_shortlist.contains(&index) {
            return Ok(workspace::Change::Unchanged(
                "That image was not in the brief.",
            ));
        }
        draft::toggle(self, index);
        Ok(workspace::Change::Applied)
    }

    fn reopen(&mut self) -> Result<workspace::Change, Fault> {
        self.open_local_library();
        self.status.clear();
        self.screen = Screen::Draft;
        self.agent_focus = Some("draft-input".into());
        Ok(workspace::Change::Applied)
    }

    fn note_write(&mut self, message: &'static str) {
        self.status = message.into();
    }
}

#[derive(Clone, Copy)]
enum OriginalError {
    Missing,
    Unreadable,
    TooLarge,
    Format,
}

impl OriginalError {
    fn import_message(self) -> &'static str {
        match self {
            Self::Missing => "This file could not be read.",
            Self::Unreadable => "This file could not be opened.",
            Self::TooLarge => "Choose an image file no larger than 24 MB.",
            Self::Format => "Choose a JPEG, PNG or WebP image.",
        }
    }

    fn restore_message(self) -> &'static str {
        match self {
            Self::Missing => "The original is unavailable.",
            Self::Unreadable => "The original could not be opened.",
            Self::TooLarge => "The original is larger than 24 MB.",
            Self::Format => "The original is not a supported image.",
        }
    }
}

fn open_original(path: &Path) -> Result<(File, u64), OriginalError> {
    let meta = path.metadata().map_err(|_| OriginalError::Missing)?;
    if !meta.is_file() {
        return Err(OriginalError::Missing);
    }
    if meta.len() > MAX_IMPORT {
        return Err(OriginalError::TooLarge);
    }
    File::open(path)
        .map(|file| (file, meta.len()))
        .map_err(|_| OriginalError::Unreadable)
}

fn original_name(path: &Path, data: &[u8]) -> Result<String, OriginalError> {
    let file_type = if data.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        return Err(OriginalError::Format);
    };
    let title = path.file_stem().map_or_else(
        || "Imported image".into(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    Ok(format!("{title}.{file_type}"))
}

/// A batch only checks each file's bounded header. Workers read image content
/// as the contact sheet or selected preview needs it.
fn validate_original(path: &Path) -> Result<String, OriginalError> {
    let (file, _) = open_original(path)?;
    let mut header = Vec::with_capacity(12);
    file.take(12)
        .read_to_end(&mut header)
        .map_err(|_| OriginalError::Unreadable)?;
    original_name(path, &header)
}

struct OriginalFile {
    filename: String,
    bytes: Arc<[u8]>,
}

fn read_original(path: &Path) -> Result<OriginalFile, OriginalError> {
    let (file, len) = open_original(path)?;
    let mut data = Vec::with_capacity(len as usize);
    file.take(MAX_IMPORT + 1)
        .read_to_end(&mut data)
        .map_err(|_| OriginalError::Unreadable)?;
    if data.len() as u64 > MAX_IMPORT {
        return Err(OriginalError::TooLarge);
    }
    Ok(OriginalFile {
        filename: original_name(path, &data)?,
        bytes: data.into(),
    })
}

const DUPLICATE_IMPORT: &str = "That image is already in your local library.";

struct ImportBatch {
    images: Vec<(PathBuf, String)>,
    total: usize,
    first_error: Option<&'static str>,
    full: bool,
}

impl ImportBatch {
    fn read(paths: Vec<PathBuf>, existing: Vec<PathBuf>) -> Self {
        let available = local::MAX_REFS.saturating_sub(existing.len());
        let mut batch = Self {
            images: Vec::new(),
            total: paths.len(),
            first_error: None,
            full: false,
        };
        for path in paths {
            if batch.images.len() == available {
                batch.full = true;
                batch
                    .first_error
                    .get_or_insert("The local library is full.");
                break;
            }
            let image = local::canonical(&path)
                .ok_or(OriginalError::Missing.import_message())
                .and_then(|path| {
                    if existing.contains(&path)
                        || batch.images.iter().any(|(added, _)| added == &path)
                    {
                        return Err(DUPLICATE_IMPORT);
                    }
                    validate_original(&path)
                        .map(|image| (path, image))
                        .map_err(OriginalError::import_message)
                });
            match image {
                Ok(image) => batch.images.push(image),
                Err(reason) => {
                    batch.first_error.get_or_insert(reason);
                }
            }
        }
        batch
    }
}

#[derive(Clone, Copy)]
struct Ink {
    bg: Color,
    rail: Color,
    panel: Color,
    field: Color,
    stage: Color,
    line: Color,
    primary: Color,
    muted: Color,
    accent: Color,
    on_accent: Color,
}

fn c(value: u32) -> Color {
    let decode = |component| rune::color::srgb_decode(component as f32 / 255.0);
    Color {
        r: decode((value >> 16) & 255),
        g: decode((value >> 8) & 255),
        b: decode(value & 255),
        a: 1.0,
    }
}

impl Ink {
    fn new(light: bool) -> Self {
        let hex = if light {
            [
                0xf5efe9, 0xede4dc, 0xf9f3ee, 0xe5d9d3, 0xe5d9d3, 0xcbbaad, 0x2a202c, 0x635465,
                0x983f2b, 0xffffff,
            ]
        } else {
            [
                0x1b1821, 0x211d29, 0x251f2d, 0x352c3e, 0x14111c, 0x423749, 0xfff0ec, 0xc4b3c5,
                0xffac86, 0x2a1a22,
            ]
        };
        let [
            bg,
            rail,
            panel,
            field,
            stage,
            line,
            primary,
            muted,
            accent,
            on_accent,
        ] = hex.map(c);
        Self {
            bg,
            rail,
            panel,
            field,
            stage,
            line,
            primary,
            muted,
            accent,
            on_accent,
        }
    }
}

fn label(value: impl Into<String>, size: f32, weight: f32, color: Color) -> El {
    text(value.into())
        .font(TextStyle::new(size, weight))
        .color(color)
}

fn action(
    key: &str,
    name: &str,
    p: Ink,
    filled: bool,
    handler: impl Fn(&mut Studio, &mut Tx) + 'static,
) -> El {
    let (background, foreground) = if filled {
        (p.accent, p.on_accent)
    } else {
        (p.field, p.primary)
    };
    el().key(key.to_owned())
        .role("button")
        .label(name.to_owned())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .cursor(Cursor::Pointer)
        .h(36.0)
        .px(12.0)
        .radius(7.0)
        .center()
        .fill(background)
        .hover(if filled { p.accent.alpha(0.82) } else { p.line })
        .child(label(name, 12.0, 600.0, foreground))
        .on_press_tx(handler)
}

fn nav(
    key: &str,
    name: &str,
    active: bool,
    p: Ink,
    run: impl Fn(&mut Studio, &mut Tx) + 'static,
) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .h(36.0)
        .items_center()
        .px(10.0)
        .gap(9.0)
        .role("button")
        .label(name.to_owned())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .cursor(Cursor::Pointer)
        .radius(7.0)
        .fill(if active { p.field } else { p.rail })
        .hover(p.field)
        .child(el().size(4.0, 16.0).radius(2.0).fill(if active {
            p.accent
        } else {
            Color::TRANSPARENT
        }))
        .child(label(
            name,
            12.0,
            550.0,
            if active { p.primary } else { p.muted },
        ))
        .on_press_tx(run)
}

fn file_open(s: &mut Studio, tx: &mut Tx) {
    if s.import_origin.is_some() {
        s.status = "Wait for the current import to finish.".into();
        return;
    }
    let options = FileDialogOptions {
        title: "Import an image".into(),
        ..Default::default()
    }
    .filter("Images", &["jpg", "jpeg", "png", "webp"]);
    tx.open_file(options, |s: &mut Studio, answer, _| match answer {
        Ok(Some(path)) => s.import(vec![path]),
        Ok(None) => (),
        Err(_) => s.status = "The system could not show the Open dialog.".into(),
    });
}

/// Explicit recovery for a saved original that moved: the dialog shows the
/// path because the user asked to locate it; no path enters the UI copy.
fn locate(s: &mut Studio, tx: &mut Tx) {
    let Some(previous) = s
        .selected
        .and_then(|i| s.assets.get(i))
        .and_then(Print::path)
        .map(Path::to_path_buf)
    else {
        return;
    };
    let options = FileDialogOptions {
        title: "Locate the original".into(),
        ..Default::default()
    }
    .filter("Images", &["jpg", "jpeg", "png", "webp"]);
    tx.open_file(options, move |s: &mut Studio, answer, tx| match answer {
        Ok(Some(path)) => s.relocate(&previous, &path, tx),
        Ok(None) => (),
        Err(_) => s.status = "The system could not show the Open dialog.".into(),
    });
}

fn export(s: &mut Studio, tx: &mut Tx) {
    if !matches!(s.local_export, LocalExport::Idle) {
        return;
    }
    let Some(asset) = s.selected.and_then(|i| s.assets.get(i)) else {
        return;
    };
    let Some(picture) = s.original(asset) else {
        s.status = if asset.available() {
            "Wait for the original to finish loading."
        } else {
            "The original is unavailable. Refresh or locate it first."
        }
        .into();
        return;
    };
    let filename = asset
        .path()
        .and_then(|path| s.local_images.original(path))
        .map_or_else(|| asset.filename.clone(), |image| image.filename.clone());
    let bytes = picture.bytes.clone();
    s.local_export = LocalExport::ChooseDestination;
    s.status.clear();
    tx.save_file(
        FileDialogOptions {
            title: "Export original".into(),
            file_name: Some(filename),
            ..Default::default()
        },
        move |s: &mut Studio, answer, _| match answer {
            Ok(Some(path)) => match OriginalWrite::start(path, bytes, s.wake.clone()) {
                Ok(write) => s.local_export = LocalExport::WriteOriginal(write),
                Err(_) => {
                    s.local_export = LocalExport::Idle;
                    s.status = "The original could not be saved. Try again.".into();
                }
            },
            Ok(None) => s.local_export = LocalExport::Idle,
            Err(_) => {
                s.local_export = LocalExport::Idle;
                s.status = "The system could not show the Save dialog.".into();
            }
        },
    );
}

fn poll(s: &mut Studio, time: f64, tx: &mut Tx) -> bool {
    let hosted_updated = hosted::poll(s, time, tx);
    let imported = s.finish_import(tx);
    let images_updated = s.poll_local_images();
    let draft_updated = s.working.due(time);
    if draft_updated {
        s.save_draft();
    }
    let mut agent_updated = false;
    if s.ids_migrated {
        s.ids_migrated = false;
        s.save_draft();
        agent_updated = true;
    }
    if let Some(mut call) = s.bridge.take() {
        let request = std::mem::take(&mut call.request);
        let mut response = workspace::evaluate_value(s, request);
        if s.agent_save {
            s.agent_save = false;
            if !draft::persist(s) {
                response["kind"] = serde_json::json!("accepted");
                response["message"] =
                    serde_json::json!("Changed in the app. The local save failed.");
            }
        }
        if let Some(key) = s.agent_focus.take() {
            if s.screen == Screen::Workspace {
                s.reveal_selection(tx);
            }
            tx.focus(key, true);
        }
        call.answer(response);
        agent_updated = true;
    }
    let changed = hosted_updated || imported || images_updated || draft_updated || agent_updated;
    let LocalExport::WriteOriginal(write) = &s.local_export else {
        return changed;
    };
    let Some(result) = write.take() else {
        return changed;
    };
    s.local_export = LocalExport::Idle;
    s.status = match result {
        Ok(_) => "Original exported.".into(),
        Err(_) => "The original could not be saved. Try another folder.".into(),
    };
    true
}

fn export_action(s: &Studio, key: &'static str, p: Ink) -> El {
    match s.local_export {
        LocalExport::Idle => action(key, "Export original", p, true, export),
        LocalExport::ChooseDestination => {
            label("Choose where to save the original…", 12.0, 500.0, p.muted).wrap()
        }
        LocalExport::WriteOriginal(_) => label("Writing original…", 12.0, 500.0, p.muted).wrap(),
    }
}

fn finish_writes(s: &mut Studio, _: &mut Tx) {
    s.bridge.stop();
    s.agent.revoke();
    if s.working.needs_save() || s.agent_save {
        s.agent_save = false;
        s.save_draft();
    }
    s.local_export = LocalExport::Idle;
    s.hosted.finish_write();
}

fn photo(s: &Studio, asset: &Print, fit: SurfaceFit, p: Ink) -> El {
    render_photo(
        s.original(asset).map(|picture| &picture.image),
        asset.available(),
        fit,
        false,
        p,
    )
}

fn thumbnail(s: &Studio, asset: &Print, p: Ink) -> El {
    let image = asset
        .path()
        .and_then(|path| s.local_images.thumbnail(path))
        .or_else(|| asset.picture().map(|picture| &picture.image));
    render_photo(image, asset.available(), SurfaceFit::Fill, true, p)
}

fn render_photo(
    image: Option<&Image>,
    available: bool,
    fit: SurfaceFit,
    thumbnail: bool,
    p: Ink,
) -> El {
    let stage = el().w_full().h_full().fill(p.stage).clip_rounded(10.0);
    if !available {
        let (message, size) = if thumbnail {
            ("Missing", 10.0)
        } else {
            ("Original unavailable", 12.0)
        };
        return stage.center().child(label(message, size, 500.0, p.muted));
    }
    if image.is_some_and(|image| image.error().is_some()) {
        let (message, size) = if thumbnail {
            ("Unreadable", 10.0)
        } else {
            ("This image could not be opened.", 14.0)
        };
        stage.center().child(label(message, size, 500.0, p.muted))
    } else if let Some(image) = image.filter(|image| image.is_ready()) {
        stage.pixels_fit(image.pixels(), fit)
    } else if thumbnail {
        stage
    } else {
        stage
            .center()
            .child(label("Loading image…", 13.0, 500.0, p.muted))
    }
}

fn rail(s: &Studio, p: Ink) -> El {
    let heading = row()
        .gap(9.0)
        .items_center()
        .child(
            el().size(24.0, 24.0)
                .radius(12.0)
                .border(1.5, p.accent)
                .center()
                .child(el().size(7.0, 7.0).radius(4.0).fill(p.accent)),
        )
        .child(label("Prints", 23.0, 650.0, p.primary));
    col()
        .key("navigation")
        .w(164.0)
        .h_full()
        .padding(15.0)
        .gap(6.0)
        .fill(p.rail)
        .border_sides([0.0, 1.0, 0.0, 0.0], p.line)
        .child(heading)
        .child(el().h(18.0))
        .child(nav(
            "creative-archive",
            "Creative Archive",
            s.shelf == Shelf::Archive && s.screen == Screen::Workspace,
            p,
            |s, _| s.show_shelf(Shelf::Archive),
        ))
        .child(nav(
            "afterlight",
            "Afterlight",
            s.shelf == Shelf::Samples && s.screen == Screen::Workspace,
            p,
            |s, _| s.show_shelf(Shelf::Samples),
        ))
        .child(nav(
            "favourites",
            "Favourites",
            s.shelf == Shelf::Favourites && s.screen == Screen::Workspace,
            p,
            |s, _| s.show_shelf(Shelf::Favourites),
        ))
        .child(nav(
            "imports",
            "Local imports",
            s.shelf == Shelf::Imports && s.screen == Screen::Workspace,
            p,
            |s, _| s.show_shelf(Shelf::Imports),
        ))
        .child(nav(
            "draft",
            "Prompt draft",
            s.screen == Screen::Draft,
            p,
            |s, _| {
                s.open_local_library();
                s.status.clear();
                s.screen = Screen::Draft;
            },
        ))
        .child(nav(
            "hosted-library",
            "Prints library",
            s.screen == Screen::Hosted,
            p,
            |s, _| {
                s.screen = if s.hosted.connected() {
                    Screen::Hosted
                } else {
                    Screen::Connection
                }
            },
        ))
        .child(el().grow(1.0))
        .child(nav(
            "theme",
            if s.theme_light {
                "Dark appearance"
            } else {
                "Light appearance"
            },
            false,
            p,
            |s, _| s.toggle_theme(),
        ))
        .child(nav(
            "connection",
            "Connect Prints",
            s.screen == Screen::Connection,
            p,
            |s, _| {
                s.status.clear();
                s.screen = Screen::Connection;
            },
        ))
        .child(nav(
            "agent-access",
            "Agent access",
            s.screen == Screen::Agent,
            p,
            |s, _| s.open_agent(),
        ))
        .child(
            row()
                .gap(7.0)
                .items_center()
                .px(4.0)
                .pt(8.0)
                .child(el().size(6.0, 6.0).radius(3.0).fill(p.accent))
                .child(label(
                    if s.hosted.connected() {
                        "Prints connected"
                    } else {
                        "Offline workspace"
                    },
                    11.0,
                    550.0,
                    p.muted,
                )),
        )
}

fn inspector(s: &Studio, asset: &Print, p: Ink) -> El {
    let size = s
        .original(asset)
        .and_then(|picture| picture.image.intrinsic_size())
        .map(|[w, h]| format!("{} × {} px", w as u32, h as u32));
    let dimensions = size.unwrap_or_else(|| {
        if asset.available() {
            asset.filename.clone()
        } else {
            "Original unavailable".into()
        }
    });
    let is_favourite = s.selected.is_some_and(|i| s.favourites.contains(&i));
    let details = col()
        .gap(10.0)
        .child(label("Image details", 12.0, 650.0, p.muted))
        .child(label(&asset.title, 23.0, 640.0, p.primary).wrap());
    let source = if let Some(edition) = asset.edition() {
        edition.source
    } else if asset.study().is_some() {
        "Afterlight · Sample artwork"
    } else if asset.available() {
        "Local import · Linked original"
    } else {
        "Local import · Original unavailable"
    };
    let mut details = details
        .child(label(dimensions, 12.0, 500.0, p.muted).truncate_middle())
        .child(el().w_full().h(1.0).fill(p.line))
        .child(label(source, 12.0, 550.0, p.muted));
    if let Some(study) = asset.study() {
        details = details
            .child(label("Creative brief", 13.0, 650.0, p.primary))
            .child(label(study.prompt, 13.0, 450.0, p.muted).wrap())
            .child(
                action("use-prompt", "Use in draft", p, false, |s, tx| {
                    s.use_prompt(tx)
                })
                .w_full(),
            );
    }
    if let Some(edition) = asset.edition() {
        details = details
            .child(label(edition.note, 13.0, 450.0, p.muted).wrap())
            .child(
                action(
                    "published-work",
                    "View published work",
                    p,
                    false,
                    move |_, tx| {
                        let _ = tx.open_url(edition.url);
                    },
                )
                .w_full(),
            );
    }
    let mut actions = col()
        .key("inspector")
        .w(252.0)
        .h_full()
        .gap(12.0)
        .padding(16.0)
        .fill(p.panel)
        .border_sides([0.0, 0.0, 0.0, 1.0], p.line)
        .child(
            col()
                .key("details-scroll")
                .w_full()
                .grow(1.0)
                .shrink()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(details),
        )
        .child(action(
            "favourite",
            if is_favourite {
                "Remove favourite"
            } else {
                "Add to favourites"
            },
            p,
            false,
            |s, _| s.favourite(),
        ))
        .child(draft::action_for_selection(s, p));
    if s.original(asset).is_some() {
        actions = actions.child(export_action(s, "export", p));
    }
    if asset.path().is_some() {
        if !asset.available() {
            actions = actions
                .child(action(
                    "refresh-original",
                    "Refresh original",
                    p,
                    false,
                    |s, tx| s.refresh(tx),
                ))
                .child(action(
                    "locate-original",
                    "Locate original…",
                    p,
                    false,
                    locate,
                ));
        }
        actions = actions.child(action(
            "remove-local",
            "Remove from library",
            p,
            false,
            |s, _| s.remove_local(),
        ));
    }
    actions
}

fn contact_page(visible: &[usize], selected: Option<usize>) -> std::ops::Range<usize> {
    let position = selected
        .and_then(|index| visible.iter().position(|&item| item == index))
        .unwrap_or(0);
    let start = position / CONTACT_PAGE_SIZE * CONTACT_PAGE_SIZE;
    start..(start + CONTACT_PAGE_SIZE).min(visible.len())
}

fn workspace(s: &Studio, p: Ink) -> El {
    let selection = s.selected.and_then(|i| s.assets.get(i));
    let visible = s.visible();
    let (title, subtitle) = match s.shelf {
        Shelf::Archive => (
            "Loftwah Creative Archive",
            "Published work from LoftwahFM, Fighter and Bubbles.",
        ),
        Shelf::Samples => ("Afterlight", "Six studies in the light that lingers."),
        Shelf::Favourites => ("Favourites", "The prints you want to return to."),
        Shelf::Imports => (
            "Local imports",
            "Your originals, linked from this device and kept between launches.",
        ),
    };
    let header = row()
        .items_center()
        .gap(8.0)
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .gap(4.0)
                .child(label(title, 31.0, 650.0, p.primary).wrap())
                .child(label(subtitle, 14.0, 450.0, p.muted).wrap()),
        )
        .child(action("import", "Import", p, false, file_open));
    let summary = if s.searching() {
        format!(
            "{} {} found",
            visible.len(),
            if visible.len() == 1 {
                "print"
            } else {
                "prints"
            }
        )
    } else {
        String::new()
    };
    let find = row()
        .w_full()
        .items_center()
        .gap(8.0)
        .child(label("Find", 11.0, 650.0, p.muted))
        .child(
            text_input(&s.search.input, TextStyle::new(13.0, 450.0))
                .key("search-input")
                .label("Find in this shelf")
                .single_line()
                .w(240.0)
                .h(36.0)
                .px(12.0)
                .py(9.0)
                .radius(7.0)
                .fill(p.field)
                .focus_ring(2.0, 2.0, p.accent)
                .color(p.primary)
                .placeholder("Title, note or filename", p.muted)
                .on_edit(|s: &mut Studio| &mut s.search.input)
                .on_input_key(|s: &mut Studio, event: &KeyEvent, tx: &mut Tx| {
                    if event.key != Key::Escape {
                        return false;
                    }
                    s.clear_search();
                    tx.blur();
                    true
                })
                .on_input_change(|s: &mut Studio, tx: &mut Tx| s.search_changed(tx)),
        )
        .child(label(summary, 11.0, 500.0, p.muted).grow(1.0).truncate());
    let body = col()
        .key("gallery")
        .w_full()
        .h_full()
        .grow(1.0)
        .shrink()
        .padding(18.0)
        .gap(13.0)
        .child(header)
        .child(find);
    let Some(asset) = selection else {
        let message: String = if s.status.is_empty() {
            if s.searching() {
                format!(
                    "No prints in {} match “{}”. Clear the find field to see the shelf.",
                    title,
                    s.search.input.text.trim()
                )
            } else {
                match s.shelf {
                    Shelf::Favourites => {
                        "Add a favourite from a collection or local import to find it here.".into()
                    }
                    Shelf::Imports => "Drop a JPEG, PNG or WebP here, or choose Import.".into(),
                    Shelf::Samples => "Loading Afterlight…".into(),
                    Shelf::Archive => "Opening the Creative Archive…".into(),
                }
            }
        } else {
            s.status.clone()
        };
        return body.child(
            el().grow(1.0)
                .w_full()
                .fill(p.stage)
                .radius(10.0)
                .center()
                .child(label(message, 14.0, 500.0, p.muted).wrap()),
        );
    };
    let view = photo(s, asset, SurfaceFit::Fit, p);
    let stage = el()
        .key("featured-image")
        .role("button")
        .label(format!("Open full view of {}", asset.title))
        .grow(1.0)
        .shrink()
        .w_full()
        .min_h(150.0)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .cursor(Cursor::Pointer)
        .child(view)
        .on_press_tx(|s: &mut Studio, tx| s.open_preview(tx, "featured-image"));
    let mut caption = row().gap(8.0).items_center().child(
        col()
            .grow(1.0)
            .shrink()
            .min_w(0.0)
            .gap(3.0)
            .child(label(&asset.title, 19.0, 620.0, p.primary).truncate())
            .child(label(comparison::caption(s, asset), 12.0, 450.0, p.muted).truncate()),
    );
    if let Some(compare) = comparison::control(s, asset, p) {
        caption = caption.child(compare);
    }
    caption = caption
        .child(action("preview", "Full view", p, false, |s, tx| {
            s.open_preview(tx, "preview")
        }))
        .child(action(
            "inspector-toggle",
            if s.inspector {
                "Hide details"
            } else {
                "Show details"
            },
            p,
            false,
            |s, _| s.inspector = !s.inspector,
        ));
    let mut strip = row().key("contact-sheet").w_full().gap(7.0).role("listbox");
    let page = contact_page(&visible, s.selected);
    for &i in &visible[page.clone()] {
        let width = if visible.len() == 1 { 170.0 } else { f32::MAX };
        let tile = &s.assets[i];
        let chosen = s.selected == Some(i);
        strip = strip.child(
            col()
                .key(format!("print-{i}"))
                .max_w(width)
                .grow(1.0)
                .shrink()
                .min_w(72.0)
                .gap(4.0)
                .padding(3.0)
                .radius(8.0)
                .fill(if chosen { p.field } else { p.bg })
                .border(1.0, if chosen { p.accent } else { p.line })
                .hover(p.field)
                .role("option")
                .label(format!("Select {}", tile.title))
                .focus_selected(chosen)
                .press_on_return()
                .focusable(true)
                .focus_ring(2.0, 1.0, p.accent)
                .cursor(Cursor::Pointer)
                .child(el().w_full().h(60.0).child(thumbnail(s, tile, p)))
                .child(
                    label(
                        &tile.title,
                        10.5,
                        550.0,
                        if chosen { p.primary } else { p.muted },
                    )
                    .truncate(),
                )
                .on_press(move |s: &mut Studio| {
                    s.selected = Some(i);
                    s.status.clear();
                    s.touch();
                }),
        );
    }
    let footer = row()
        .gap(8.0)
        .items_center()
        .child(
            label(
                if s.status.is_empty() {
                    "Arrow keys to browse  ·  Enter for full view"
                } else {
                    &s.status
                },
                11.0,
                500.0,
                p.muted,
            )
            .grow(1.0)
            .truncate(),
        )
        .child(label(
            format!(
                "{} {}",
                visible.len(),
                if visible.len() == 1 {
                    "image"
                } else {
                    "images"
                }
            ),
            11.0,
            550.0,
            p.muted,
        ));
    let strip = if visible.len() > AFTERLIGHT.len() {
        strip.scroll_x()
    } else {
        strip
    };
    let mut body = body.child(stage).child(caption).child(strip);
    if visible.len() > CONTACT_PAGE_SIZE {
        let mut navigation = row().gap(8.0).items_center().justify_end();
        if page.start > 0 {
            navigation = navigation.child(action(
                "previous-page",
                "Previous page",
                p,
                false,
                |s, tx| {
                    s.turn_contact_page(false, tx);
                },
            ));
        }
        navigation = navigation.child(label(
            format!(
                "Page {} of {}",
                page.start / CONTACT_PAGE_SIZE + 1,
                visible.len().div_ceil(CONTACT_PAGE_SIZE)
            ),
            11.0,
            550.0,
            p.muted,
        ));
        if page.end < visible.len() {
            navigation = navigation.child(action("next-page", "Next page", p, false, |s, tx| {
                s.turn_contact_page(true, tx);
            }));
        }
        body = body.child(navigation);
    }
    body.child(footer)
}

fn fullscreen(s: &Studio, p: Ink) -> El {
    let Some(asset) = s.selected.and_then(|i| s.assets.get(i)) else {
        return workspace(s, p);
    };
    let close = action(
        "close-preview",
        "Back to contact sheet",
        p,
        false,
        Studio::close_preview,
    );
    col()
        .key("full-view")
        .w_full()
        .h_full()
        .padding(20.0)
        .gap(13.0)
        .fill(p.bg)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_center()
                .child(
                    label(&asset.title, 22.0, 640.0, p.primary)
                        .grow(1.0)
                        .truncate(),
                )
                .child(viewer::controls(&s.viewer, p, s.viewer.active(), false))
                .child(close),
        )
        .child(el().w_full().grow(1.0).shrink().child(viewer::stage(
            &s.viewer,
            s.original(asset).map(|picture| &picture.image),
            asset.available(),
            p,
            0,
        )))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .child(action("previous-print", "Previous", p, false, |s, _| {
                    s.step(-1)
                }))
                .child(action("next-print", "Next", p, false, |s, _| s.step(1)))
                .child(
                    label(
                        if s.status.is_empty() {
                            "Arrow keys to browse  ·  Esc to close"
                        } else {
                            &s.status
                        },
                        12.0,
                        470.0,
                        p.muted,
                    )
                    .grow(1.0),
                )
                .child(export_action(s, "export-preview", p)),
        )
}

/// Native, discoverable opt-in. The bearer is deliberately not shown here:
/// token presentation belongs to the coordinator's local bridge.
fn agent_panel(s: &Studio, p: Ink) -> El {
    let access = s.agent_access();
    let (state, state_color) = match access.mode() {
        None => ("Off", p.muted),
        Some(AccessMode::Read) => ("Read only", p.accent),
        Some(AccessMode::Edit) => ("Can edit", p.accent),
    };
    let mut controls = col().w_full().gap(8.0);
    match access.mode() {
        None => {
            controls = controls
                .child(
                    action(
                        "agent-enable-read",
                        "Enable read access",
                        p,
                        true,
                        |s, _| {
                            let _ = s.agent_activate(AccessMode::Read);
                        },
                    )
                    .w_full(),
                )
                .child(
                    action(
                        "agent-enable-edit",
                        "Enable edit access",
                        p,
                        false,
                        |s, _| {
                            let _ = s.agent_activate(AccessMode::Edit);
                        },
                    )
                    .w_full(),
                );
        }
        Some(AccessMode::Read) => {
            controls = controls
                .child(
                    label(
                        "Read access: the agent can discover the workspace and read the brief and image identity.",
                        13.0,
                        450.0,
                        p.muted,
                    )
                    .wrap(),
                )
                .child(
                    action("agent-allow-edits", "Allow edits", p, false, |s, _| {
                        let _ = s.agent_activate(AccessMode::Edit);
                    })
                    .w_full(),
                );
        }
        Some(AccessMode::Edit) => {
            controls = controls
                .child(
                    label(
                        "Edit access: the agent can select, compare, edit the brief and keep or remove references.",
                        13.0,
                        450.0,
                        p.muted,
                    )
                    .wrap(),
                )
                .child(
                    action("agent-read-only", "Switch to read only", p, false, |s, _| {
                        let _ = s.agent_activate(AccessMode::Read);
                    })
                    .w_full(),
                );
        }
    }
    if access.mode().is_some() {
        controls = controls
            .child(
                action("agent-copy-grant", "Copy grant path", p, true, |s, tx| {
                    if let Some(path) = s.bridge.grant_path().and_then(Path::to_str) {
                        tx.copy(path.to_owned());
                        s.status =
                            "Grant path copied. Share it only with the local agent you’ve chosen."
                                .into();
                    }
                })
                .w_full(),
            )
            .child(
                action("agent-revoke", "Revoke access now", p, false, |s, _| {
                    s.agent_revoke()
                })
                .w_full(),
            );
    }
    col()
        .key("agent-view")
        .w_full()
        .h_full()
        .grow(1.0)
        .padding(22.0)
        .gap(14.0)
        .child(label("Agent access", 36.0, 650.0, p.primary))
        .child(
            label(
                "Share this brief, the archive and your selected or shortlisted local references with a local agent.",
                14.0,
                450.0,
                p.muted,
            )
            .wrap(),
        )
        .child(
            row()
                .gap(10.0)
                .items_center()
                .child(label("Status", 12.0, 650.0, p.muted))
                .child(label(state, 13.0, 650.0, state_color)),
        )
        .child(el().w_full().h(1.0).fill(p.line))
        .child(controls)
        .child(label(&s.status, 12.0, 500.0, p.muted).wrap())
}

fn drafting(s: &Studio, p: Ink) -> El {
    col()
        .key("draft-view")
        .w_full()
        .h_full()
        .grow(1.0)
        .padding(22.0)
        .gap(16.0)
        .child(label("Prompt draft", 36.0, 650.0, p.primary))
        .child(
            label(
                "Keep your brief and image choices together.",
                14.0,
                450.0,
                p.muted,
            )
            .wrap(),
        )
        .child(
            text_input(&s.draft, TextStyle::new(16.0, 450.0))
                .key("draft-input")
                .label("Prompt draft")
                .w_full()
                .h(188.0)
                .padding(16.0)
                .radius(10.0)
                .fill(p.field)
                .focus_ring(2.0, 2.0, p.accent)
                .color(p.primary)
                .wrap()
                .placeholder("Describe the image you want to make…", p.muted)
                .on_edit(|s: &mut Studio| &mut s.draft)
                .on_input_change(|s: &mut Studio, tx| {
                    s.touch();
                    s.working.pending(tx)
                }),
        )
        .child(
            row()
                .gap(8.0)
                .items_center()
                .child(action("save-draft", "Save draft", p, true, |s, _| {
                    s.save_draft()
                }))
                .child(action("copy-draft", "Copy draft", p, false, |s, tx| {
                    if !s.draft.text.trim().is_empty() {
                        tx.copy(s.draft.text.clone());
                        s.status = "Draft copied.".into();
                    }
                }))
                .child(action("clear-draft", "Clear draft", p, false, draft::clear))
                .children(draft::restore_control(s, p))
                .child(label(&s.status, 12.0, 500.0, p.muted).wrap()),
        )
        .child(label(s.working.status(), 12.0, 500.0, p.muted).wrap())
        .child(draft::shortlist(s, p))
}

fn view(s: &Studio, cx: &mut Cx<Studio>) -> El {
    s.viewer.set_device_scale(cx.env().scale);
    let p = Ink::new(s.theme_light);
    let mut shell = row().key("workspace").h_full().w_full().fill(p.bg);
    if s.screen == Screen::Preview {
        return shell.child(fullscreen(s, p));
    }
    if let Screen::Compare(pair) = s.screen {
        return shell.child(comparison::view(s, pair, p));
    }
    shell = shell.child(rail(s, p));
    match s.screen {
        Screen::Workspace => {
            shell = shell.child(workspace(s, p));
            if s.inspector
                && let Some(asset) = s.selected.and_then(|i| s.assets.get(i))
            {
                shell = shell.child(inspector(s, asset, p));
            }
        }
        Screen::Draft => shell = shell.child(drafting(s, p)),
        Screen::Connection => shell = shell.child(hosted::connection(s, p)),
        Screen::Hosted => shell = shell.child(hosted::library(s, p)),
        Screen::Agent => shell = shell.child(agent_panel(s, p)),
        Screen::Preview | Screen::Compare(_) => unreachable!(),
    }
    shell.on_drop_files(|s: &mut Studio, paths, _| {
        if s.screen == Screen::Workspace {
            s.import(paths.to_vec());
        }
    })
}

fn keyboard(s: &mut Studio, event: &KeyEvent, tx: &mut Tx) -> bool {
    let inspecting = matches!(s.screen, Screen::Preview | Screen::Compare(_));
    match event.key {
        Key::Escape
            if s.screen == Screen::Workspace
                && (tx.focused() == Some("search-input") || s.searching()) =>
        {
            s.clear_search();
            tx.blur();
        }
        Key::Escape if matches!(s.screen, Screen::Compare(_)) => {
            comparison::return_to_workspace(s, tx);
        }
        Key::Escape if s.screen == Screen::Preview => s.close_preview(tx),
        Key::Escape if s.screen != Screen::Workspace => {
            s.screen = Screen::Workspace;
            tx.blur();
        }
        Key::Command('f') if matches!(s.screen, Screen::Workspace | Screen::Preview) => {
            s.focus_search(tx);
        }
        Key::Command('o') => file_open(s, tx),
        Key::Command('s') if s.screen == Screen::Draft => s.save_draft(),
        Key::Command('s') if matches!(s.screen, Screen::Workspace | Screen::Preview) => {
            export(s, tx)
        }
        // Zoom and pan keys are inspection-only. They move the camera but never blur the
        // button that has keyboard focus, so Tab and Return still work on the controls.
        Key::Command('=') | Key::Command('+') if inspecting => {
            let pane = s.viewer.active();
            s.viewer.zoom_in(pane, tx)
        }
        Key::Command('-') | Key::Command('_') if inspecting => {
            let pane = s.viewer.active();
            s.viewer.zoom_out(pane, tx)
        }
        Key::Command('0') if inspecting => {
            let pane = s.viewer.active();
            s.viewer.fit(pane, tx)
        }
        Key::Left | Key::Right | Key::Up | Key::Down
            if inspecting && s.viewer.zoomed(s.viewer.active()) =>
        {
            let pane = s.viewer.active();
            let delta = match event.key {
                Key::Left => (-1.0, 0.0),
                Key::Right => (1.0, 0.0),
                Key::Up => (0.0, -1.0),
                Key::Down => (0.0, 1.0),
                _ => (0.0, 0.0),
            };
            s.viewer.pan_key(pane, delta.0, delta.1, tx);
        }
        Key::Left | Key::Right if matches!(s.screen, Screen::Workspace | Screen::Preview) => {
            let focused_tile = tx.focused().is_some_and(|key| key.starts_with("print-"));
            s.step(if event.key == Key::Left { -1 } else { 1 });
            if s.screen == Screen::Workspace {
                s.reveal_selection(tx);
                if focused_tile && let Some(index) = s.selected {
                    tx.focus(format!("print-{index}"), true);
                }
            }
        }
        Key::Enter
            if s.screen == Screen::Workspace && s.selected.is_some() && tx.focused().is_none() =>
        {
            s.open_preview(tx, "featured-image")
        }
        _ => return false,
    }
    true
}

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(status) = agent_bridge::cli(&arguments) {
        std::process::exit(status);
    }
    if let Err(error) = tooling_mode::prepare(arguments) {
        eprintln!("Prints verification profile: {error}");
        std::process::exit(2);
    }
    rune::set_app_id(APP_ID);
    let _ = rune::set_app_icon(include_bytes!("../assets/icon.png").to_vec());
    let app = App::new(AppSpec {
        min_size: Some((940.0, 660.0)),
        setup: Some(Studio::load),
        poll: Some(poll),
        on_key: Some(keyboard),
        scenes: &[("light", |s| s.theme_light = true)],
        ..AppSpec::new("Prints Desktop", (1350.0, 830.0), Studio::new, view)
    })
    .on_close(finish_writes)
    .on_quit(finish_writes)
    .chrome(rune::Chrome {
        full_size_content: false,
        ..Default::default()
    });
    navigation::items()
        .into_iter()
        .fold(app, App::menu_item)
        .run();
}

#[cfg(test)]
mod tests {
    use super::{Search, TextEdit};
    use rune::{Cx, El, Env, Runtime, el, row};

    fn matches_terms(fields: &[&str], query: &str) -> bool {
        let mut search = Search {
            input: TextEdit::new(query),
            ..Search::default()
        };
        search.update();
        search.includes(&fields.join("\n").to_lowercase())
    }

    #[test]
    fn search_ignores_case_and_matches_every_word_across_fields() {
        let fields = [
            "Relay Station",
            "An antenna tower holds its ground.",
            "relay-station.jpg",
        ];
        assert!(matches_terms(&fields, "RELAY station"));
        assert!(matches_terms(&fields, "tower relay"));
        assert!(matches_terms(&fields, "  station   jpg "));
    }

    #[test]
    fn search_requires_all_words_and_an_empty_query_matches_everything() {
        let fields = ["Salt Memory", "Fractured pink salt.", "salt-memory.jpg"];
        assert!(matches_terms(&fields, ""));
        assert!(matches_terms(&fields, "   "));
        assert!(!matches_terms(&fields, "salt harbour"));
        assert!(!matches_terms(&fields, "relay"));
    }

    /// The contact sheet uses the role/state pair Rune maps to native selection. This checks the
    /// element tree Rune feeds to the accessibility layer, not a spoken screen-reader result.
    #[test]
    fn selected_tile_reports_selection_state_in_the_element_tree() {
        fn view(_: &(), _: &mut Cx<()>) -> El {
            let tile = |key: &'static str, selected: bool| {
                el().key(key)
                    .role("option")
                    .label(format!("Select {key}"))
                    .focusable(true)
                    .focus_selected(selected)
                    .press_on_return()
                    .size(100.0, 30.0)
            };
            row()
                .key("sheet")
                .role("listbox")
                .child(tile("tile-0", true))
                .child(tile("tile-1", false))
        }
        let mut runtime = Runtime::new((), view, (300.0, 100.0), Env::new(true, 1.0));
        runtime.frame(0.0);
        let elements = runtime.elements();
        let selected = |key: &str| {
            elements
                .iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap()
                .item_selected
        };
        assert!(selected("tile-0"));
        assert!(!selected("tile-1"));
    }
}
