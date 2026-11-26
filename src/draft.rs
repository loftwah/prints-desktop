use rune::{El, ScrollSpec, Tx, col, row};

use crate::{
    Ink, Print, Shelf, Studio, action,
    connected_asset::Identity,
    label,
    workspace::{AssetId, WorkspaceId},
};

const BUNDLED_KEY: &str = "draft-bundled-shortlist";
pub(crate) const SHORTLIST_LIMIT: usize = 12;

/// A shortlist entry keeps an opaque asset id alongside its legacy locator.
/// The locator is only used for snapshots saved before ids existed.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct DraftAsset {
    #[serde(default)]
    pub id: AssetId,
    #[serde(flatten)]
    kind: DraftKind,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
enum DraftKind {
    Bundled(String),
    Linked(#[serde(with = "crate::local::native_path")] std::path::PathBuf),
    /// A hosted original is identified by opaque provenance, never by a path
    /// or a display name. The flattened tag keeps older snapshots readable.
    Hosted(Identity),
}

impl DraftAsset {
    fn bundled(name: String, id: AssetId) -> Self {
        Self {
            id,
            kind: DraftKind::Bundled(name),
        }
    }

    fn linked(path: std::path::PathBuf, id: AssetId) -> Self {
        Self {
            id,
            kind: DraftKind::Linked(path),
        }
    }

    fn hosted(identity: Identity, id: AssetId) -> Self {
        Self {
            id,
            kind: DraftKind::Hosted(identity),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct Workspace {
    pub text: String,
    cleared_text: Option<String>,
    pub library: Vec<crate::local::Reference>,
    pub shortlist: Vec<DraftAsset>,
    /// persists in the local snapshot and scopes every agent write
    #[serde(default)]
    pub workspace_id: WorkspaceId,
}

#[derive(Default)]
enum SaveState {
    #[default]
    Idle,
    Saving(f64),
    Saved,
    Failed,
}

pub(crate) struct Store {
    file: rune::prefs::Prefs,
    pub loaded: Option<Workspace>,
    pub workspace_id: WorkspaceId,
    cleared_text: Option<String>,
    state: SaveState,
}

impl Store {
    pub(crate) fn open(prefs: &rune::prefs::Prefs) -> Self {
        let mut file = prefs
            .path()
            .map_or_else(rune::prefs::Prefs::memory, |path| {
                rune::prefs::Prefs::at(path.with_extension("draft-workspace.json"))
            });
        let mut loaded = file.get::<Workspace>("workspace");
        let mut workspace_id = loaded
            .as_ref()
            .map_or_else(WorkspaceId::default, |saved| saved.workspace_id.clone());
        if workspace_id.is_empty() {
            workspace_id = WorkspaceId::current();
            if let Some(saved) = &mut loaded {
                saved.workspace_id = workspace_id.clone();
                let _ = file.set("workspace", &*saved);
            }
        }
        let cleared_text = loaded.as_ref().and_then(|saved| saved.cleared_text.clone());
        let state = if loaded.is_some() || prefs.contains("draft") {
            SaveState::Saved
        } else {
            SaveState::Idle
        };
        Self {
            file,
            loaded,
            workspace_id,
            cleared_text,
            state,
        }
    }

    pub(crate) fn pending(&mut self, tx: &mut Tx) {
        let due = tx.clock() + 0.4;
        self.state = SaveState::Saving(due);
        tx.wake_at(due);
    }

    pub(crate) fn due(&self, time: f64) -> bool {
        matches!(self.state, SaveState::Saving(due) if time >= due)
    }

    pub(crate) fn needs_save(&self) -> bool {
        matches!(self.state, SaveState::Saving(_) | SaveState::Failed)
    }

    fn write(&mut self, workspace: &Workspace) -> bool {
        let saved = self.file.set("workspace", workspace).is_ok();
        self.state = if saved {
            SaveState::Saved
        } else {
            SaveState::Failed
        };
        saved
    }

    pub(crate) fn status(&self) -> &'static str {
        match self.state {
            SaveState::Idle => "Changes save automatically.",
            SaveState::Saving(_) => "Saving draft…",
            SaveState::Saved if self.file.is_persistent() => "Draft saved on this device.",
            SaveState::Saved => "Draft saved for this session.",
            SaveState::Failed => "Couldn’t save the draft. Choose Save draft to retry.",
        }
    }
}

/// Returns true when a legacy shortlist needed ids, so the caller can save the
/// migrated snapshot once the local library has been opened.
pub(crate) fn restore_bundled(studio: &mut Studio) -> bool {
    let legacy: Vec<String> = studio.prefs.get(BUNDLED_KEY).unwrap_or_default();
    let mut entries: Vec<DraftAsset> = if let Some(saved) = &studio.working.loaded {
        saved.shortlist.clone()
    } else {
        legacy
            .into_iter()
            .map(|name| {
                let id = studio
                    .assets
                    .iter()
                    .find(|print| {
                        (print.edition().is_some() || print.study().is_some())
                            && print.filename == name
                    })
                    .map_or_else(
                        || AssetId::bundled("legacy", &name),
                        |print| print.id().clone(),
                    );
                DraftAsset::bundled(name, id)
            })
            .collect()
    };
    // Saved hosted entries need a placeholder before the shortlist is resolved.
    // Append in entry order so bundled and local indices never shift.
    let mut migrated = false;
    for entry in entries.iter_mut().take(SHORTLIST_LIMIT) {
        if let DraftKind::Hosted(identity) = &entry.kind {
            let id = identity.id(&studio.working.workspace_id);
            migrated |= entry.id != id;
            entry.id = id;
        }
        if let DraftKind::Hosted(identity) = &entry.kind
            && !entry.id.is_empty()
            && !studio.assets.iter().any(|print| print.id() == &entry.id)
        {
            studio
                .assets
                .push(Print::locked(identity.clone(), entry.id.clone()));
        }
    }
    for entry in &mut entries {
        if entry.id.is_empty()
            && let Some(index) = resolve_one(&studio.assets, entry)
        {
            entry.id = studio.assets[index].id().clone();
            migrated = true;
        }
    }
    studio.draft_shortlist = resolve(&studio.assets, &entries);
    if let Some(saved) = &mut studio.working.loaded {
        saved.shortlist = entries;
    }
    migrated
}

fn resolve_one(assets: &[Print], entry: &DraftAsset) -> Option<usize> {
    if !entry.id.is_empty() {
        return assets.iter().position(|print| print.id() == &entry.id);
    }
    assets.iter().position(|print| match &entry.kind {
        DraftKind::Bundled(name) => {
            (print.edition().is_some() || print.study().is_some()) && print.filename == *name
        }
        DraftKind::Linked(path) => print.path() == Some(path.as_path()),
        // An id-less hosted entry has no safe fallback: identities are opaque.
        DraftKind::Hosted(_) => false,
    })
}

pub(crate) fn resolve(assets: &[Print], entries: &[DraftAsset]) -> Vec<usize> {
    let mut result = Vec::new();
    for entry in entries {
        if let Some(index) = resolve_one(assets, entry).filter(|index| !result.contains(index)) {
            result.push(index);
            if result.len() == SHORTLIST_LIMIT {
                break;
            }
        }
    }
    result
}

pub(crate) fn persist(studio: &mut Studio) -> bool {
    studio.open_local_library();
    let library = studio
        .assets
        .iter()
        .enumerate()
        .filter_map(|(index, image)| {
            Some(crate::local::Reference {
                id: image.id().clone(),
                provenance: image.provenance().cloned(),
                path: image.path()?.to_path_buf(),
                favourite: studio.favourites.contains(&index),
                in_draft: false,
            })
        })
        .collect();
    let shortlist = studio
        .draft_shortlist
        .iter()
        .filter_map(|&index| {
            let print = studio.assets.get(index)?;
            if let Some(identity) = print.connected() {
                return Some(DraftAsset::hosted(identity.clone(), print.id().clone()));
            }
            Some(print.path().map_or_else(
                || DraftAsset::bundled(print.filename.clone(), print.id().clone()),
                |path| DraftAsset::linked(path.to_path_buf(), print.id().clone()),
            ))
        })
        .collect();
    studio.working.write(&Workspace {
        text: studio.draft.text.clone(),
        cleared_text: studio.working.cleared_text.clone(),
        library,
        shortlist,
        workspace_id: studio.working.workspace_id.clone(),
    })
}

pub(crate) fn clear(studio: &mut Studio, tx: &mut Tx) {
    if !studio.draft.text.is_empty() {
        studio.working.cleared_text = Some(std::mem::take(&mut studio.draft.text));
        studio.touch();
        studio.draft = rune::TextEdit::new("");
        studio.working.pending(tx);
    }
}

pub(crate) fn restore_control(studio: &Studio, ink: Ink) -> Option<El> {
    (studio.draft.text.is_empty() && studio.working.cleared_text.is_some()).then(|| {
        action(
            "restore-cleared-draft",
            "Undo clear",
            ink,
            false,
            |studio, tx| {
                if let Some(text) = studio.working.cleared_text.take() {
                    studio.draft = rune::TextEdit::new(text);
                    studio.touch();
                    studio.working.pending(tx);
                }
            },
        )
    })
}

pub(crate) fn toggle(studio: &mut Studio, index: usize) {
    studio.open_local_library();
    if studio.assets.get(index).is_none() {
        return;
    }
    studio.touch();
    let kept = if let Some(position) = studio
        .draft_shortlist
        .iter()
        .position(|&item| item == index)
    {
        studio.draft_shortlist.remove(position);
        false
    } else {
        if studio.draft_shortlist.len() >= SHORTLIST_LIMIT {
            studio.status =
                format!("This draft holds {SHORTLIST_LIMIT} images. Remove one to add another.");
            return;
        }
        studio.draft_shortlist.push(index);
        true
    };
    let saved = persist(studio);
    studio.status = match (saved, kept) {
        (true, true) => "Kept with draft.",
        (true, false) => "Removed from draft.",
        (false, _) => "The draft shortlist could not be saved. Changes apply for this session.",
    }
    .into();
}

pub(crate) fn forget_asset(studio: &mut Studio, index: usize) -> bool {
    let kept = studio.draft_shortlist.contains(&index);
    studio.draft_shortlist.retain(|&item| item != index);
    for item in &mut studio.draft_shortlist {
        if *item > index {
            *item -= 1;
        }
    }
    kept
}

fn open(studio: &mut Studio, index: usize, tx: &mut Tx) {
    // Hosted prints (live or locked placeholders) are opened through the
    // connected handoff. It owns the available/full and reconnect status.
    if studio
        .assets
        .get(index)
        .is_some_and(|print| print.connected().is_some())
    {
        studio.open_connected_reference(index, tx);
        return;
    }
    let Some(print) = studio.assets.get(index) else {
        return;
    };
    let shelf = if print.path().is_some() {
        Shelf::Imports
    } else if print.study().is_some() {
        Shelf::Samples
    } else {
        Shelf::Archive
    };
    studio.show_shelf(shelf);
    studio.selected = Some(index);
    studio.reveal_selection(tx);
    tx.focus("featured-image", true);
}

pub(crate) fn action_for_selection(studio: &Studio, ink: Ink) -> El {
    let title = if studio
        .selected
        .is_some_and(|index| studio.draft_shortlist.contains(&index))
    {
        "Remove from draft"
    } else {
        "Keep with draft"
    };
    action("keep-with-draft", title, ink, false, |studio, _| {
        if let Some(index) = studio.selected {
            toggle(studio, index);
        }
    })
}

pub(crate) fn shortlist(studio: &Studio, ink: Ink) -> El {
    let mut list = col()
        .key("draft-shortlist")
        .w_full()
        .grow(1.0)
        .shrink()
        .gap(6.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if studio.draft_shortlist.is_empty() {
        list = list.child(
            label(
                "Choose Keep with draft in an image’s details to add it here.",
                13.0,
                450.0,
                ink.muted,
            )
            .wrap(),
        );
    }
    for &index in &studio.draft_shortlist {
        if let Some(print) = studio.assets.get(index) {
            list = list.child(shortlist_row(print, index, ink));
        }
    }
    col()
        .w_full()
        .grow(1.0)
        .shrink()
        .gap(6.0)
        .child(label("Draft shortlist", 17.0, 650.0, ink.primary))
        .child(list)
}

fn shortlist_row(print: &Print, index: usize, ink: Ink) -> El {
    // Locked hosted placeholders carry generic names, so the title shown here
    // is safe text and the existing Open/Remove controls stay available.
    row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(
            label(&print.title, 14.0, 550.0, ink.primary)
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .truncate(),
        )
        .child(
            action(
                &format!("draft-open-{index}"),
                if print.connected().is_some() && !print.available() {
                    "Find in Prints"
                } else {
                    "Open"
                },
                ink,
                false,
                move |studio, tx| {
                    open(studio, index, tx);
                },
            )
            .label(format!("Open {}", print.title)),
        )
        .child(
            action(
                &format!("draft-remove-{index}"),
                "Remove",
                ink,
                false,
                move |studio, tx| {
                    toggle(studio, index);
                    tx.blur();
                },
            )
            .label(format!("Remove {} from draft", print.title)),
        )
}
