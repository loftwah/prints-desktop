# Prints Desktop v0.4.1 benchmark baseline

25 November 2026 AEDT. [Flagship benchmark #4](https://github.com/loftwah/prints-desktop/issues/4) owns the wider evaluation.

**Qualification.** The offline native preview is public. The connected preview has live read and original-export evidence on one Oyster installation. Creative and cross-platform acceptance are still open.

- [Desktop v0.4.1](https://github.com/loftwah/prints-desktop/releases/tag/v0.4.1), source `1e5714a7ed2a19ebad4cda4141eb03def8851a31`, Rune `e725e066548688ad1bb1f38ce64ae73f06ec0968`.
- [Public site](https://prints.loftwah.com/), Composer source `d43203a3161c0d7c495ddba77013fa24d6e77cbc`, Cloudflare Worker `e7a2c824-9071-457c-b893-a9b3c37c832e`.
- This run downloaded the four archives and checksums while signed out. All SHA-256 checks passed. The extracted Mac bundle identified the exact source and passed strict ad-hoc signature verification. Linux archives contained ARM64 and x86-64 ELF binaries. The Windows archive contained an x86-64 PE GUI executable.
- The new linked-original fixture ran at 940 × 660 on macOS 26.6.2, Apple M4 Pro, with a locked debug build offline. The tracked source diff from v0.4.1 to the fixture checkout was documentation only. Physical Mac and graphical Debian results below are from the [release’s runtime verification](https://github.com/loftwah/prints-desktop/issues/4#issuecomment-7152711348).

## Journey coverage

| ID | Status | Evidence or next check |
| --- | --- | --- |
| PD-01 · Discover and download | Pass | At 390 px, the homepage menu led to Downloads and the installation guide. The four package links and four checksum links targeted v0.4.1. The launch snapshot and live counts opened at 390 and 1440 px. |
| PD-02 · Open offline | Pass on Apple Silicon | The released app opened on the Mac with the Creative Archive populated. Keep cold and warm native launch timings for a follow-up run. |
| PD-03 · Find and export | Pass on Apple Silicon | Native Find and a byte-exact Fighter PNG export through the Save dialog. Find and Escape also ran in the two Debian sessions. |
| PD-04 · Keep local work | Partial | Existing Rune replays exercised import, drafts, restart, relocation and removal. This run reproduced the 12-link cap. Physical restart, moved-file recovery and keyboard-only use still need coverage. |
| PD-05 · Compare and select | Partial | The LoftwahFM cover and poster were compared in the shipped UI. Favourite persistence has replay evidence. Larger creator libraries and reuse across projects remain open. |
| PD-06 · Connect an installation | Partial | A prior scoped key opened six real outputs, exported matching bytes and failed after revocation. Model discovery returned 502 on that host. Current model discovery and platform credential-store coverage remain open. [Live read receipt](https://github.com/loftwah/app-composer/issues/1292#issuecomment-7149505628) |
| PD-07 · Create natively | Blocked | Prints generation requires an admitted actor and a Computer. Named App keys resolve as Team-key actors and the generation methods refuse them. Web Prints remains the supported handoff. [Scoped creation #6](https://github.com/loftwah/prints-desktop/issues/6) |
| PD-08 · Recover real work | Partial | Local restart and linked-file recovery have replay evidence. Pending paid-job recovery and same-request reconciliation depend on PD-07. |
| PD-09 · Work with other media | Partial | The workshop demonstrates a previous image workflow using Oyster MCP and Workers AI. Native video and audio creation, playback and export remain unqualified. [Workshop proof](https://prints.loftwah.com/prints-desktop/workshop-20261125/proof.json) |
| PD-10 · Use native distributions | Partial | Apple Silicon ran physically. Two Debian X11 sessions exercised Find, one under x64 translation. Windows x64 is an unsigned cross-build with no Windows desktop session here. [Windows #7](https://github.com/loftwah/prints-desktop/issues/7) |
| PD-11 · Follow an independent setup | Partial | The site exposes the account-free offline path and a documented [Oyster setup](https://docs.fungi.computer/guides/run-your-own-oyster). An independent owner’s first useful output and live Gateway trace remain open. [Adoption #997](https://github.com/loftwah/app-composer/issues/997) |
| PD-12 · Produce and publish work | Partial | The published [native walkthrough](https://prints.loftwah.com/watch) shows comparison, selection and exact export against a real portfolio release. The [public study](https://prints.loftwah.com/make) records its artwork separately from prior Prints-generated outputs. |

Three journeys pass within the stated platform scope, eight are partial and one is blocked. The proposed flagship weighting needs comparable native timing, accessibility, independent setup and authorised creation evidence before an overall score would be meaningful.

## Linked originals

The offline archive has five [published portfolio editions](../assets/archive/README.md) and a separate Afterlight shelf with six studies. The LoftwahFM release gives comparison a real purpose. Fighter and Bubbles establish different subjects and provenance. The set still covers only three portfolio projects. The archive manifest records unknown settings and credits external or other-tool sources.

| Linked paths in preferences | Originals loaded | Three wall-clock times (ms) |
| ---: | ---: | --- |
| 12 | 12 | 3927, 1379, 1383 |
| 30 | 12 | 1368, 1369, 1372 |
| 300 | 12 | 1368, 1370, 1370 |
| 3,000 | 12 | 1371, 1376, 1367 |

Each path pointed to the same public JPEG through a different filesystem entry. The Rune replay opened Imports at 0.2 s and exported the view tree at 0.7 s. Each trial started a fresh process with isolated preferences. Paths 1–12 appeared and path 13 was absent in all 12 trials. The wall clock includes process startup, the scripted wait and tree output. These results establish the supported capacity. They leave startup latency, scrolling performance and large-library decoding unmeasured.

For the next comparable run, keep the same fixture and laptop, record the first usable frame and search response separately, and report loaded count before comparing times. Use the existing [archive](../scripts/check-archive.py) and [local library](../scripts/check-local-library.py) replays for comparison, Find, exports and restart behavior. A larger shelf needs bounded thumbnail decoding and selected full-resolution loading before raising the [current limit](https://github.com/loftwah/prints-desktop/blob/v0.4.1/src/local.rs). Test Windows and the scoped upstream grant independently of that work.
