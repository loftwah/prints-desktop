use rune::{MenuItem, Tx};

use crate::{Screen, Shelf, Studio, export, file_open};

fn open_shelf(s: &mut Studio, tx: &mut Tx, shelf: Shelf) {
    s.show_shelf(shelf);
    tx.blur();
    s.reveal_selection(tx);
}

pub(crate) fn items() -> Vec<MenuItem<Studio>> {
    vec![
        MenuItem::new("Import Original…", file_open).menu("File"),
        MenuItem::new("Export Original…", |s: &mut Studio, tx| {
            if matches!(s.screen, Screen::Workspace | Screen::Preview) {
                export(s, tx);
            }
        })
        .menu("File"),
        MenuItem::new("Creative Archive", |s, tx| {
            open_shelf(s, tx, Shelf::Archive)
        })
        .menu("Library"),
        MenuItem::new("Afterlight", |s, tx| open_shelf(s, tx, Shelf::Samples)).menu("Library"),
        MenuItem::new("Favourites", |s, tx| open_shelf(s, tx, Shelf::Favourites)).menu("Library"),
        MenuItem::new("Local Imports", |s, tx| open_shelf(s, tx, Shelf::Imports)).menu("Library"),
        MenuItem::separator().menu("Library"),
        MenuItem::new("Agent access…", |s: &mut Studio, _| s.open_agent()).menu("Library"),
        MenuItem::new("Connect Prints", |s: &mut Studio, tx| {
            s.status.clear();
            s.screen = Screen::Connection;
            tx.blur();
        })
        .menu("Library"),
        MenuItem::new("Find in Shelf…", |s: &mut Studio, tx| s.focus_search(tx))
            .menu("Edit")
            .key_equivalent("cmd+f".parse().expect("valid Find shortcut")),
        MenuItem::new("Show or Hide Details", |s: &mut Studio, _| {
            s.inspector = !s.inspector;
        })
        .menu("View"),
        MenuItem::new("Switch Appearance", |s: &mut Studio, _| s.toggle_theme()).menu("View"),
        MenuItem::new("Prints Desktop Website", |_, tx| {
            let _ = tx.open_url("https://prints.loftwah.com/");
        })
        .menu("Help"),
        MenuItem::new("Oyster Connection Guide", |_, tx| {
            let _ =
                tx.open_url("https://github.com/loftwah/prints-desktop/blob/main/docs/oyster.md");
        })
        .menu("Help"),
    ]
}
