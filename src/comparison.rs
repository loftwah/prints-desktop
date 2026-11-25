use rune::{El, SurfaceFit, Tx, col, el, row};

use crate::{Ink, Print, Screen, Studio, action, label, photo};

const CAMPAIGN_PRINTS: [&str; 2] = ["higher-than-before.jpg", "higher-than-before-poster.jpg"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pair {
    Campaign,
    Local(usize, usize),
}

fn open(s: &mut Studio, tx: &mut Tx, pair: Pair) {
    s.screen = Screen::Compare(pair);
    tx.focus("close-comparison", tx.by_keys() || tx.focus_visible());
}

pub(crate) fn return_to_workspace(s: &mut Studio, tx: &mut Tx) {
    let key = match s.screen {
        Screen::Compare(Pair::Campaign) => "compare-editions",
        Screen::Compare(Pair::Local(..)) => "compare-local",
        _ => return,
    };
    s.screen = Screen::Workspace;
    tx.focus(key, tx.by_keys() || tx.focus_visible());
}

fn first_local(s: &Studio) -> Option<usize> {
    let path = s.comparison_first.as_deref()?;
    s.assets
        .iter()
        .position(|asset| asset.path() == Some(path) && asset.available())
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
    let path = asset.path()?.to_path_buf();
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
                s.comparison_first = Some(path.clone());
                tx.focus("clear-comparison", tx.by_keys() || tx.focus_visible());
            },
        ),
        Some(first) if first == current => {
            action("clear-comparison", "Clear comparison", p, false, |s, tx| {
                s.comparison_first = None;
                tx.focus("choose-comparison", tx.by_keys() || tx.focus_visible());
            })
        }
        Some(first) => action("compare-local", "Compare images", p, false, move |s, tx| {
            open(s, tx, Pair::Local(first, current));
        }),
    };
    Some(button)
}

fn panel(s: &Studio, asset: &Print, title: &str, p: Ink) -> El {
    col()
        .grow(1.0)
        .shrink()
        .gap(8.0)
        .min_w(0.0)
        .child(label(title, 18.0, 650.0, p.primary).truncate())
        .child(
            el().w_full()
                .grow(1.0)
                .shrink()
                .child(photo(s, asset, SurfaceFit::Fit, p)),
        )
        .child(label(&asset.filename, 12.0, 500.0, p.muted).truncate_middle())
}

pub(crate) fn view(s: &Studio, pair: Pair, p: Ink) -> El {
    let (key, title, description, images) = match pair {
        Pair::Campaign => (
            "campaign-comparison",
            "Higher Than Before",
            "One release, two formats. Compare the artwork at its original proportions.",
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
            "Both originals shown at their own proportions.",
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
                .child(panel(s, first, first_title, p))
                .child(panel(s, second, second_title, p))
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
                    col()
                        .grow(1.0)
                        .gap(4.0)
                        .child(label(title, 27.0, 650.0, p.primary))
                        .child(label(description, 13.0, 450.0, p.muted).wrap()),
                )
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
