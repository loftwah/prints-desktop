use rune::{El, ScrollSpec, Tx, col, row};

use crate::{Ink, Print, Shelf, Studio, action, label};

const BUNDLED_KEY: &str = "draft-bundled-shortlist";
const SHORTLIST_LIMIT: usize = 12;

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) enum DraftAsset {
    Bundled(String),
    Linked(#[serde(with = "crate::local::native_path")] std::path::PathBuf),
}

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct Workspace {
    pub text: String,
    cleared_text: Option<String>,
    pub library: Vec<crate::local::Reference>,
    pub shortlist: Vec<DraftAsset>,
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
    cleared_text: Option<String>,
    state: SaveState,
}

impl Store {
    pub(crate) fn open(prefs: &rune::prefs::Prefs) -> Self {
        let file = prefs
            .path()
            .map_or_else(rune::prefs::Prefs::memory, |path| {
                rune::prefs::Prefs::at(path.with_extension("draft-workspace.json"))
            });
        let loaded = file.get::<Workspace>("workspace");
        let cleared_text = loaded.as_ref().and_then(|saved| saved.cleared_text.clone());
        let state = if loaded.is_some() || prefs.contains("draft") {
            SaveState::Saved
        } else {
            SaveState::Idle
        };
        Self {
            file,
            loaded,
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

pub(crate) fn restore_bundled(studio: &mut Studio) {
    let legacy: Vec<String> = studio.prefs.get(BUNDLED_KEY).unwrap_or_default();
    if let Some(saved) = &studio.working.loaded {
        studio.draft_shortlist = resolve(&studio.assets, &saved.shortlist);
    } else {
        let entries: Vec<_> = legacy.into_iter().map(DraftAsset::Bundled).collect();
        studio.draft_shortlist = resolve(&studio.assets, &entries);
    }
}

pub(crate) fn resolve(assets: &[Print], entries: &[DraftAsset]) -> Vec<usize> {
    let mut result = Vec::new();
    for entry in entries {
        let index = assets.iter().position(|print| match entry {
            DraftAsset::Bundled(name) => print.path().is_none() && print.filename == *name,
            DraftAsset::Linked(path) => print.path() == Some(path.as_path()),
        });
        if let Some(index) = index.filter(|index| !result.contains(index)) {
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
            Some(print.path().map_or_else(
                || DraftAsset::Bundled(print.filename.clone()),
                |path| DraftAsset::Linked(path.to_path_buf()),
            ))
        })
        .collect();
    studio.working.write(&Workspace {
        text: studio.draft.text.clone(),
        cleared_text: studio.working.cleared_text.clone(),
        library,
        shortlist,
    })
}

pub(crate) fn clear(studio: &mut Studio, tx: &mut Tx) {
    if !studio.draft.text.is_empty() {
        studio.working.cleared_text = Some(std::mem::take(&mut studio.draft.text));
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
                "Open",
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
