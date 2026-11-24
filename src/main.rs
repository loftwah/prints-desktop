#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use rune::prefs::Prefs;
use rune::tok::TextStyle;
use rune::{
    App, AppSpec, Color, Cursor, Cx, El, FileDialogOptions, Image, ImageCache, Key, KeyEvent,
    ScrollSpec, SurfaceFit, TextEdit, Tx, Waker, col, el, row, text, text_input,
};

const APP_ID: &str = "com.loftwah.prints-desktop";
const MAX_IMPORT: u64 = 24 * 1024 * 1024;

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
    Samples,
    Favourites,
    Imports,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Workspace,
    Preview,
    Draft,
    Connection,
}

struct Print {
    title: String,
    filename: String,
    bytes: Arc<[u8]>,
    image: Image,
    study: Option<usize>,
}

struct Studio {
    shelf: Shelf,
    screen: Screen,
    assets: Vec<Print>,
    selected: Option<usize>,
    favourites: Vec<usize>,
    theme_light: bool,
    inspector: bool,
    draft: TextEdit,
    draft_dirty: bool,
    status: String,
    prefs: Prefs,
    cache: Option<ImageCache>,
    proof: bool,
}

impl Studio {
    fn new() -> Self {
        let proof = cfg!(debug_assertions)
            && std::env::args()
                .any(|arg| matches!(arg.as_str(), "--snap" | "--tree" | "--script" | "--video"));
        let prefs = if proof {
            Prefs::memory()
        } else {
            Prefs::open(APP_ID)
        };
        let mut favourites: Vec<usize> = prefs.get("favourites").unwrap_or_default();
        favourites.retain(|&i| i < AFTERLIGHT.len());
        favourites.sort_unstable();
        favourites.dedup();
        let theme_light = prefs.get("light").unwrap_or(false);
        let draft = TextEdit::new(prefs.get::<String>("draft").unwrap_or_default());
        Self {
            shelf: Shelf::Samples,
            screen: Screen::Workspace,
            assets: Vec::new(),
            selected: Some(0),
            favourites,
            theme_light,
            inspector: true,
            draft,
            draft_dirty: false,
            status: String::new(),
            prefs,
            cache: None,
            proof,
        }
    }

    fn load(&mut self, wake: Waker) {
        let cache = ImageCache::new(wake);
        self.assets = AFTERLIGHT
            .iter()
            .enumerate()
            .map(|(index, study)| {
                let bytes: Arc<[u8]> = study.bytes.into();
                let image = cache.load(bytes.clone());
                Print {
                    title: study.title.into(),
                    filename: study.filename.into(),
                    bytes,
                    image,
                    study: Some(index),
                }
            })
            .collect();
        if self.proof {
            for asset in &self.assets {
                let _ = asset.image.wait();
            }
        }
        self.cache = Some(cache);
    }

    fn visible(&self) -> Vec<usize> {
        self.assets
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let visible = match self.shelf {
                    Shelf::Samples => item.study.is_some(),
                    Shelf::Favourites => self.favourites.contains(&i),
                    Shelf::Imports => item.study.is_none(),
                };
                visible.then_some(i)
            })
            .collect()
    }

    fn show_shelf(&mut self, shelf: Shelf) {
        self.shelf = shelf;
        self.screen = Screen::Workspace;
        self.status.clear();
        let visible = self.visible();
        if !self.selected.is_some_and(|i| visible.contains(&i)) {
            self.selected = visible.first().copied();
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
    }

    fn favourite(&mut self) {
        let Some(i) = self.selected else { return };
        if self.favourites.contains(&i) {
            self.favourites.retain(|&value| value != i);
        } else {
            self.favourites.push(i);
        }
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

    fn import(&mut self, path: &Path) -> Result<(), &'static str> {
        let Some(cache) = &self.cache else {
            return Err("The library is starting. Try again in a moment.");
        };
        let meta = path
            .metadata()
            .map_err(|_| "This file could not be read.")?;
        if !meta.is_file() || meta.len() > MAX_IMPORT {
            return Err("Choose an image file no larger than 24 MB.");
        }
        if self.assets.len() >= AFTERLIGHT.len() + 12 {
            return Err("Start a fresh session to import more than 12 images.");
        }
        let file = File::open(path).map_err(|_| "This file could not be opened.")?;
        let mut data = Vec::new();
        file.take(MAX_IMPORT + 1)
            .read_to_end(&mut data)
            .map_err(|_| "This file could not be read.")?;
        if data.len() as u64 > MAX_IMPORT {
            return Err("Choose an image file no larger than 24 MB.");
        }
        let file_type = if data.starts_with(&[0xff, 0xd8, 0xff]) {
            "jpg"
        } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            "png"
        } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
            "webp"
        } else {
            return Err("Choose a JPEG, PNG or WebP image.");
        };
        let title = path.file_stem().map_or_else(
            || "Imported image".into(),
            |s| s.to_string_lossy().into_owned(),
        );
        let filename = format!("{title}.{file_type}");
        let bytes: Arc<[u8]> = data.into();
        let image = cache.load(bytes.clone());
        self.assets.push(Print {
            title,
            filename,
            bytes,
            image,
            study: None,
        });
        self.selected = Some(self.assets.len() - 1);
        self.show_shelf(Shelf::Imports);
        Ok(())
    }

    fn use_prompt(&mut self) {
        let Some(study) = self
            .selected
            .and_then(|i| self.assets.get(i))
            .and_then(|a| a.study)
        else {
            return;
        };
        if self.draft.text.trim().is_empty() {
            self.draft = TextEdit::new(AFTERLIGHT[study].prompt);
            self.draft_dirty = true;
            self.status.clear();
        } else {
            self.status = "Your draft is still here. Clear it first to use this brief.".into();
        }
        self.screen = Screen::Draft;
    }

    fn save_draft(&mut self) {
        if !self.draft_dirty {
            return;
        }
        match self.prefs.set("draft", &self.draft.text) {
            Ok(()) => {
                self.draft_dirty = false;
                self.status = "Draft saved on this device.".into();
            }
            Err(_) => self.status = "The draft could not be saved to disk.".into(),
        }
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

fn file_open(_: &mut Studio, tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Import an image".into(),
        ..Default::default()
    }
    .filter("Images", &["jpg", "jpeg", "png", "webp"]);
    tx.open_file(options, |s: &mut Studio, answer, _| match answer {
        Ok(Some(path)) => {
            if let Err(err) = s.import(&path) {
                s.status = err.into();
            }
        }
        Ok(None) => (),
        Err(_) => s.status = "The system could not show the Open dialog.".into(),
    });
}

fn export(s: &mut Studio, tx: &mut Tx) {
    let Some(asset) = s.selected.and_then(|i| s.assets.get(i)) else {
        return;
    };
    let filename = asset.filename.clone();
    let bytes = asset.bytes.clone();
    tx.save_file(
        FileDialogOptions {
            title: "Export original".into(),
            file_name: Some(filename),
            ..Default::default()
        },
        move |s: &mut Studio, answer, _| match answer {
            Ok(Some(path)) => match write_original(&path, &bytes) {
                Ok(()) => s.status = "Original exported.".into(),
                Err(_) => s.status = "The original could not be saved. Try another folder.".into(),
            },
            Ok(None) => (),
            Err(_) => s.status = "The system could not show the Save dialog.".into(),
        },
    );
}

fn write_original(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let identifier = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_file_name(format!(".prints-{}-{identifier}.tmp", std::process::id()));
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let installed = written.and_then(|_| std::fs::rename(&temporary, path));
    if installed.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    installed
}

fn photo(asset: &Print, fit: SurfaceFit, p: Ink) -> El {
    let stage = el().w_full().h_full().fill(p.stage).clip_rounded(10.0);
    if asset.image.error().is_some() {
        stage.center().child(label(
            "This image could not be opened.",
            14.0,
            500.0,
            p.muted,
        ))
    } else if asset.image.is_ready() {
        stage.pixels_fit(asset.image.pixels(), fit)
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
        .child(el().h(26.0))
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
                s.status.clear();
                s.screen = Screen::Draft;
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
        .child(
            row()
                .gap(7.0)
                .items_center()
                .px(4.0)
                .pt(8.0)
                .child(el().size(6.0, 6.0).radius(3.0).fill(p.accent))
                .child(label("Offline workspace", 11.0, 550.0, p.muted)),
        )
}

fn inspector(s: &Studio, asset: &Print, p: Ink) -> El {
    let size = asset
        .image
        .intrinsic_size()
        .map(|[w, h]| format!("{} × {} px", w as u32, h as u32));
    let dimensions = size.unwrap_or_else(|| asset.filename.clone());
    let is_favourite = s.selected.is_some_and(|i| s.favourites.contains(&i));
    let details = col()
        .gap(10.0)
        .child(label("Image details", 12.0, 650.0, p.muted))
        .child(label(&asset.title, 23.0, 640.0, p.primary).wrap());
    let mut details = details
        .child(label(dimensions, 12.0, 500.0, p.muted).truncate_middle())
        .child(el().w_full().h(1.0).fill(p.line))
        .child(label(
            if asset.study.is_some() {
                "Afterlight · Sample artwork"
            } else {
                "Local import · This session"
            },
            12.0,
            550.0,
            p.muted,
        ));
    if let Some(study) = asset.study {
        details = details
            .child(label("Creative brief", 13.0, 650.0, p.primary))
            .child(label(AFTERLIGHT[study].prompt, 13.0, 450.0, p.muted).wrap())
            .child(
                action("use-prompt", "Use in draft", p, false, |s, _| {
                    s.use_prompt()
                })
                .w_full(),
            );
    }
    col()
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
            match (is_favourite, asset.study.is_some()) {
                (true, _) => "Remove favourite",
                (false, true) => "Add to favourites",
                (false, false) => "Favourite for this session",
            },
            p,
            false,
            |s, _| s.favourite(),
        ))
        .child(action("export", "Export original", p, true, export))
}

fn workspace(s: &Studio, p: Ink) -> El {
    let selection = s.selected.and_then(|i| s.assets.get(i));
    let (title, subtitle) = match s.shelf {
        Shelf::Samples => ("Afterlight", "Six studies in the light that lingers."),
        Shelf::Favourites => ("Favourites", "The prints you want to return to."),
        Shelf::Imports => (
            "Local imports",
            "Images opened from this device during this session.",
        ),
    };
    let header = row()
        .items_center()
        .gap(8.0)
        .child(
            col()
                .grow(1.0)
                .gap(4.0)
                .child(label(title, 36.0, 650.0, p.primary))
                .child(label(subtitle, 14.0, 450.0, p.muted).wrap()),
        )
        .child(action("import", "Import", p, false, file_open));
    let body = col()
        .key("gallery")
        .w_full()
        .h_full()
        .grow(1.0)
        .shrink()
        .padding(18.0)
        .gap(13.0)
        .child(header);
    let Some(asset) = selection else {
        let message = if s.status.is_empty() {
            match s.shelf {
                Shelf::Favourites => {
                    "Add a favourite in Afterlight or Local imports to find it here."
                }
                Shelf::Imports => "Drop a JPEG, PNG or WebP here, or choose Import.",
                Shelf::Samples => "Loading Afterlight…",
            }
        } else {
            &s.status
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
    let view = photo(asset, SurfaceFit::Fit, p);
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
        .on_press(|s: &mut Studio| s.screen = Screen::Preview);
    let mut caption = row().gap(8.0).items_center().child(
        col()
            .grow(1.0)
            .gap(3.0)
            .child(label(&asset.title, 19.0, 620.0, p.primary).truncate())
            .child(
                label(
                    asset
                        .study
                        .map_or("Original from this device.", |i| AFTERLIGHT[i].note),
                    12.0,
                    450.0,
                    p.muted,
                )
                .truncate(),
            ),
    );
    caption = caption
        .child(action("preview", "Full view", p, false, |s, _| {
            s.screen = Screen::Preview
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
    let visible = s.visible();
    let mut strip = row().key("contact-sheet").w_full().gap(7.0);
    for &i in &visible {
        let tile = &s.assets[i];
        let chosen = s.selected == Some(i);
        strip = strip.child(
            col()
                .key(format!("print-{i}"))
                .grow(1.0)
                .shrink()
                .min_w(72.0)
                .gap(4.0)
                .padding(3.0)
                .radius(8.0)
                .fill(if chosen { p.field } else { p.bg })
                .border(1.0, if chosen { p.accent } else { p.line })
                .hover(p.field)
                .role("button")
                .label(format!("Select {}", tile.title))
                .focusable(true)
                .focus_ring(2.0, 1.0, p.accent)
                .cursor(Cursor::Pointer)
                .child(
                    el().w_full()
                        .h(60.0)
                        .child(photo(tile, SurfaceFit::Fill, p)),
                )
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
            format!("{} images", visible.len()),
            11.0,
            550.0,
            p.muted,
        ));
    let strip = if visible.len() > AFTERLIGHT.len() {
        strip.scroll_x()
    } else {
        strip
    };
    body.child(stage).child(caption).child(strip).child(footer)
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
        |s, tx| {
            s.screen = Screen::Workspace;
            tx.blur();
        },
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
                .child(close),
        )
        .child(
            el().w_full()
                .grow(1.0)
                .shrink()
                .child(photo(asset, SurfaceFit::Fit, p)),
        )
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
                .child(action("export-preview", "Export original", p, true, export)),
        )
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
                "Write a brief of your own, or start from an Afterlight print.",
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
                .h(220.0)
                .padding(16.0)
                .radius(10.0)
                .fill(p.field)
                .focus_ring(2.0, 2.0, p.accent)
                .color(p.primary)
                .wrap()
                .placeholder("Describe the image you want to make…", p.muted)
                .on_edit(|s: &mut Studio| {
                    s.draft_dirty = true;
                    &mut s.draft
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
                .child(action("clear-draft", "Clear draft", p, false, |s, _| {
                    s.draft = TextEdit::new("");
                    s.draft_dirty = true;
                    s.status.clear();
                }))
                .child(label(&s.status, 12.0, 500.0, p.muted).wrap()),
        )
}

fn connection(p: Ink) -> El {
    col().key("connection-view").w_full().h_full().grow(1.0).padding(22.0).gap(15.0)
        .child(label("Connect Prints", 36.0, 650.0, p.primary))
        .child(label("This preview opens the Afterlight collection and images on your device.", 16.0, 550.0, p.primary).wrap())
        .child(label("Hosted Prints access requires a compatible Oyster installation, a scoped Prints App key and an admitted Computer. Hosted generation may use provider credits and Cloudflare resources.", 14.0, 450.0, p.muted).w(500.0).wrap())
        .child(row().gap(8.0)
            .child(action("setup-guide", "Read the Prints setup guide", p, true, |_, tx| {
                let _ = tx.open_url("https://docs.fungi.computer/apps/prints");
            }))
            .child(action("return-afterlight", "Open Afterlight", p, false, |s, _| s.show_shelf(Shelf::Samples))))
}

fn view(s: &Studio, _: &mut Cx<Studio>) -> El {
    let p = Ink::new(s.theme_light);
    let mut shell = row().key("workspace").h_full().w_full().fill(p.bg);
    if s.screen == Screen::Preview {
        return shell.child(fullscreen(s, p));
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
        Screen::Connection => shell = shell.child(connection(p)),
        Screen::Preview => unreachable!(),
    }
    shell.on_drop_files(|s: &mut Studio, paths, _| {
        if let Some(path) = paths.first()
            && let Err(err) = s.import(path)
        {
            s.status = err.into();
        }
    })
}

fn keyboard(s: &mut Studio, event: &KeyEvent, tx: &mut Tx) -> bool {
    match event.key {
        Key::Escape if s.screen != Screen::Workspace => {
            s.screen = Screen::Workspace;
            tx.blur();
        }
        Key::Command('o') => file_open(s, tx),
        Key::Command('s') if s.screen == Screen::Draft => s.save_draft(),
        Key::Command('s') if matches!(s.screen, Screen::Workspace | Screen::Preview) => {
            export(s, tx)
        }
        Key::Left if matches!(s.screen, Screen::Workspace | Screen::Preview) => s.step(-1),
        Key::Right if matches!(s.screen, Screen::Workspace | Screen::Preview) => s.step(1),
        Key::Enter
            if s.screen == Screen::Workspace && s.selected.is_some() && tx.focused().is_none() =>
        {
            s.screen = Screen::Preview
        }
        _ => return false,
    }
    true
}

fn main() {
    rune::set_app_id(APP_ID);
    App::new(AppSpec {
        min_size: Some((940.0, 660.0)),
        setup: Some(Studio::load),
        on_key: Some(keyboard),
        scenes: &[("light", |s| s.theme_light = true)],
        ..AppSpec::new("Prints Desktop", (1350.0, 830.0), Studio::new, view)
    })
    .chrome(rune::Chrome {
        full_size_content: false,
        ..Default::default()
    })
    .run();
}
