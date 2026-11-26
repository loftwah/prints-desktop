use rune::{El, Tx, col, el, row};

use crate::{Ink, Print, Screen, Studio, action, label, viewer};

pub(crate) const CAMPAIGN_PRINTS: [&str; 2] =
    ["higher-than-before.jpg", "higher-than-before-poster.jpg"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pair {
    Campaign,
    Local(usize, usize),
}

pub(crate) fn start(s: &mut Studio, pair: Pair) {
    s.viewer.reset();
    s.screen = Screen::Compare(pair);
}

fn open(s: &mut Studio, tx: &mut Tx, pair: Pair) {
    start(s, pair);
    tx.focus("close-comparison", tx.by_keys() || tx.focus_visible());
}

pub(crate) fn return_to_workspace(s: &mut Studio, tx: &mut Tx) {
    let key = match s.screen {
        Screen::Compare(Pair::Campaign) => "compare-editions",
        Screen::Compare(Pair::Local(..)) => {
            match s.selected.and_then(|index| s.assets.get(index)) {
                Some(asset) if asset.path().is_some() => "compare-local",
                Some(asset)
                    if asset
                        .edition()
                        .is_some_and(|edition| CAMPAIGN_PRINTS.contains(&edition.filename)) =>
                {
                    "compare-editions"
                }
                _ => "featured-image",
            }
        }
        _ => return,
    };
    s.screen = Screen::Workspace;
    tx.focus(key, tx.by_keys() || tx.focus_visible());
}

fn first_local(s: &Studio) -> Option<usize> {
    let id = s.comparison_first.as_ref()?;
    s.assets
        .iter()
        .position(|asset| asset.id() == id && asset.available())
}

pub(crate) fn caption(s: &Studio, asset: &Print) -> String {
    if asset.path().is_some()
        && asset.available()
        && let Some(first) = first_local(s)
    {
        return if s.selected == Some(first) {
            "Select a second image to compare.".into()
        } else {
            format!("Compare with {}", s.assets[first].title)
        };
    }
    asset.caption().into()
}

pub(crate) fn control(s: &Studio, asset: &Print, p: Ink) -> Option<El> {
    if asset
        .edition()
        .is_some_and(|edition| CAMPAIGN_PRINTS.contains(&edition.filename))
    {
        return Some(action(
            "compare-editions",
            "Compare formats",
            p,
            false,
            |s, tx| open(s, tx, Pair::Campaign),
        ));
    }
    let current = s.selected?;
    asset.path()?;
    let id = asset.id().clone();
    if !asset.available() {
        return None;
    }
    let button = match first_local(s) {
        None => action(
            "choose-comparison",
            "Set first image",
            p,
            false,
            move |s, tx| {
                s.comparison_first = Some(id.clone());
                s.touch();
                tx.focus("clear-comparison", tx.by_keys() || tx.focus_visible());
            },
        ),
        Some(first) if first == current => {
            action("clear-comparison", "Clear comparison", p, false, |s, tx| {
                s.comparison_first = None;
                s.touch();
                tx.focus("choose-comparison", tx.by_keys() || tx.focus_visible());
            })
        }
        Some(first) => action("compare-local", "Compare images", p, false, move |s, tx| {
            open(s, tx, Pair::Local(first, current));
        }),
    };
    Some(button)
}

fn panel(s: &Studio, asset: &Print, title: &str, p: Ink, pane: usize) -> El {
    col()
        .grow(1.0)
        .shrink()
        .gap(8.0)
        .min_w(0.0)
        .child(label(title, 18.0, 650.0, p.primary).truncate())
        .child(el().w_full().grow(1.0).shrink().child(viewer::stage(
            &s.viewer,
            s.original(asset).map(|picture| &picture.image),
            asset.available(),
            p,
            pane,
        )))
        .child(label(&asset.filename, 12.0, 500.0, p.muted).truncate_middle())
}

pub(crate) fn view(s: &Studio, pair: Pair, p: Ink) -> El {
    let (key, title, images) = match pair {
        Pair::Campaign => (
            "campaign-comparison",
            "Higher Than Before",
            CAMPAIGN_PRINTS.map(|name| {
                s.assets.iter().find(|asset| {
                    asset
                        .edition()
                        .is_some_and(|edition| edition.filename == name)
                })
            }),
        ),
        Pair::Local(first, second) => (
            "local-comparison",
            "Compare images",
            [s.assets.get(first), s.assets.get(second)],
        ),
    };
    let panels = match images {
        [Some(first), Some(second)] => {
            let [first_title, second_title] = match pair {
                Pair::Campaign => ["Album cover", "Release poster"],
                Pair::Local(..) => [first.title.as_str(), second.title.as_str()],
            };
            row()
                .w_full()
                .grow(1.0)
                .shrink()
                .gap(15.0)
                .child(panel(s, first, first_title, p, 0))
                .child(panel(s, second, second_title, p, 1))
        }
        _ => label(
            "The comparison formats are unavailable.",
            14.0,
            500.0,
            p.muted,
        ),
    };
    col()
        .key(key)
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
                    label(title, 27.0, 650.0, p.primary)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .truncate(),
                )
                .child(viewer::controls(&s.viewer, p, s.viewer.active(), true))
                .child(action(
                    "close-comparison",
                    "Back to contact sheet",
                    p,
                    false,
                    return_to_workspace,
                )),
        )
        .child(panels)
}
