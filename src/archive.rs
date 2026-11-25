pub(crate) struct Edition {
    pub title: &'static str,
    pub note: &'static str,
    pub source: &'static str,
    pub url: &'static str,
    pub filename: &'static str,
    pub bytes: &'static [u8],
}

pub(crate) const EDITIONS: &[Edition] = &[
    Edition {
        title: "Higher Than Before",
        note: "The finished cover of LoftwahFM’s November 2026 album. An orange double-chevron tablet sits against cobalt paper.",
        source: "LoftwahFM · Album cover · ImageGen",
        url: "https://fm.loftwah.com/album/higher-than-before/",
        filename: "higher-than-before.jpg",
        bytes: include_bytes!("../assets/archive/higher-than-before.jpg"),
    },
    Edition {
        title: "Release Poster",
        note: "The same record reworked for a wall. The tall format gives the lettering and the chevron room to read at a distance.",
        source: "LoftwahFM · Release campaign · ImageGen",
        url: "https://fm.loftwah.com/album/higher-than-before/",
        filename: "higher-than-before-poster.jpg",
        bytes: include_bytes!("../assets/archive/higher-than-before-poster.jpg"),
    },
    Edition {
        title: "The Last Lift",
        note: "A drained pool and a service lift at dawn. Interior artwork from the short fiction booklet that accompanied Higher Than Before.",
        source: "LoftwahFM · Booklet artwork",
        url: "https://fm.loftwah.com/album/higher-than-before/#album-booklet",
        filename: "the-last-lift.jpg",
        bytes: include_bytes!("../assets/archive/the-last-lift.jpg"),
    },
    Edition {
        title: "Before the Bell",
        note: "A ringside study made in Media Studio from Fighter’s Across the Ring poster, published in the game’s artwork collection.",
        source: "Fighter · Poster study · FLUX.2 Klein",
        url: "https://media.loftwah.com/p/yFSN93LOFy1ja4j0",
        filename: "before-the-bell.png",
        bytes: include_bytes!("../assets/archive/before-the-bell.png"),
    },
    Edition {
        title: "One Shot",
        note: "A documented Classic board clear, framed as a Bubbles poster. The board and score come from recorded gameplay.",
        source: "Bubbles · Gameplay poster · Recorded Classic clear",
        url: "https://bubbles.loftwah.com/assets/social/bubbles-one-shot-card.png",
        filename: "bubbles-one-shot.png",
        bytes: include_bytes!("../assets/archive/bubbles-one-shot.png"),
    },
];
