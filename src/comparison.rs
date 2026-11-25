use rune::{El, SurfaceFit, col, el, row};

use crate::{AFTERLIGHT, Ink, Print, Screen, Studio, action, label, photo};

const CAMPAIGN_PRINTS: [usize; 2] = [AFTERLIGHT.len(), AFTERLIGHT.len() + 1];

pub(crate) fn applies(s: &Studio) -> bool {
    s.selected.is_some_and(|i| CAMPAIGN_PRINTS.contains(&i))
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
        .child(
            row()
                .w_full()
                .grow(1.0)
                .shrink()
                .gap(15.0)
                .child(panel(&s.assets[CAMPAIGN_PRINTS[0]], "Album cover", p))
                .child(panel(&s.assets[CAMPAIGN_PRINTS[1]], "Release poster", p)),
        )
}
