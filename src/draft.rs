use rune::{El, ScrollSpec, Tx, col, row};

use crate::{Ink, Print, Shelf, Studio, action, label};

const BUNDLED_KEY: &str = "draft-bundled-shortlist";
const SHORTLIST_LIMIT: usize = 12;

pub(crate) fn restore_bundled(studio: &mut Studio) {
    let saved = studio
        .prefs
        .get::<Vec<String>>(BUNDLED_KEY)
        .unwrap_or_default();
    studio.draft_shortlist.extend(
        studio
            .assets
            .iter()
            .enumerate()
            .filter_map(|(index, print)| {
                saved
                    .iter()
                    .any(|name| *name == print.filename)
                    .then_some(index)
            }),
    );
}

pub(crate) fn toggle(studio: &mut Studio, index: usize) {
    studio.open_local_library();
    let Some(print) = studio.assets.get(index) else {
        return;
    };
    let linked = print.path().is_some();
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
    let saved = if linked {
        studio.persist_local()
    } else {
        let names: Vec<&str> = studio
            .draft_shortlist
            .iter()
            .filter_map(|&item| {
                let print = studio.assets.get(item)?;
                print.path().is_none().then_some(print.filename.as_str())
            })
            .collect();
        studio.prefs.set(BUNDLED_KEY, &names).is_ok()
    };
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
