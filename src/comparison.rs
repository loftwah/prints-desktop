use rune::{El, SurfaceFit, col, el, row};

use crate::{Ink, Print, Screen, Studio, action, label, photo};

const CAMPAIGN_PRINTS: [&str; 2] = ["higher-than-before.jpg", "higher-than-before-poster.jpg"];

pub(crate) fn applies(s: &Studio) -> bool {
    s.selected
        .and_then(|i| s.assets.get(i))
        .and_then(Print::edition)
        .is_some_and(|edition| CAMPAIGN_PRINTS.contains(&edition.filename))
}

fn panel(asset: &Print, format: &str, p: Ink) -> El {
    col()
        .grow(1.0)
        .shrink()
        .gap(8.0)
        .child(label(format, 18.0, 650.0, p.primary))
        .child(
            el().w_full()
                .grow(1.0)
                .shrink()
                .child(photo(asset, SurfaceFit::Fit, p)),
        )
        .child(label(&asset.filename, 12.0, 500.0, p.muted))
}

pub(crate) fn view(s: &Studio, p: Ink) -> El {
    let editions = CAMPAIGN_PRINTS.map(|name| {
        s.assets.iter().find(|asset| {
            asset
                .edition()
                .is_some_and(|edition| edition.filename == name)
        })
    });
    let panels = match editions {
        [Some(cover), Some(poster)] => row()
            .w_full()
            .grow(1.0)
            .shrink()
            .gap(15.0)
            .child(panel(cover, "Album cover", p))
            .child(panel(poster, "Release poster", p)),
        _ => label(
            "The comparison formats are unavailable.",
            14.0,
            500.0,
            p.muted,
        ),
    };
    col()
        .key("campaign-comparison")
        .w_full()
        .h_full()
        .padding(20.0)
        .gap(16.0)
        .child(
            row()
                .w_full()
                .items_center()
                .gap(12.0)
                .child(
                    col()
                        .grow(1.0)
                        .gap(4.0)
                        .child(label("Higher Than Before", 27.0, 650.0, p.primary))
                        .child(label(
                            "One release, two formats. Compare the artwork at its original proportions.",
                            13.0,
                            450.0,
                            p.muted,
                        ).wrap()),
                )
                .child(action("close-comparison", "Back to contact sheet", p, false, |s, tx| {
                    s.screen = Screen::Workspace;
                    tx.blur();
                })),
        )
        .child(panels)
}
