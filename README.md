# Prints Desktop

Open **Afterlight**, six studies in the light that lingers. Browse the contact
sheet, see a print at full size, keep favourites, and export the included
JPEGs. Bring an image from your own machine or use a sample's prompt as a
starting point for an editable, local draft. Sample favourites and saved drafts
reopen on this device. Imports and their favourites last for the current session.

Prints Desktop is a standalone native Rust application built with
[Anomaly's Rune](https://github.com/anomalyco/rune). Its offline workspace
needs no App Composer account, server or inference credits. It is an
independent Loftwah project for the [Fungi](https://docs.fungi.computer/)
ecosystem.

## Download

Open the [preview release](https://github.com/loftwah/prints-desktop/releases/tag/v0.1.0)
for platform details and SHA-256 checksums. On Apple Silicon, extract the ZIP
and open `Prints Desktop.app`. The preview uses an ad-hoc signature and is
unnotarised. Check the checksum before installing. See [Apple's Gatekeeper
guidance](https://support.apple.com/en-au/102445) for the prompts macOS may
show for a downloaded preview.

See the [App Composer showcase and walkthrough](https://composer.loftwah.com/prints-desktop).

## Build and inspect

Install Rust through [rustup](https://rustup.rs/). This checkout selects Rust
1.99.1 and pins Rune and its platform patches in `Cargo.toml`. macOS requires
the Xcode command-line tools.

```sh
cargo run --locked
scripts/check-workspace.sh
cargo run --locked -- --snap prints.png --size 1360x840 --settle
scripts/package-macos.sh
```

The workspace replay requires Bash and Python 3. Packaging on Apple Silicon writes a signed local bundle, ZIP and checksum to
`dist/`. Rune's debug build also provides `--tree` and `--script` for
inspecting the window and replaying input. The bundle contains its release
version and Git revision in `Contents/Resources/build.txt`.

## Platform builds

- **Apple Silicon macOS** · `scripts/package-macos.sh` creates the app and archive.
  It targets macOS 14 or later. The release notes identify the tested OS.
- **Linux ARM64** · Build with `cargo build --locked --release` using Rust 1.99.1.
  On Debian, install `libxkbcommon-dev` and `libwayland-dev` to compile.
  A graphical session needs the X11 or Wayland client libraries and working
  graphics drivers. Native Open and Save use the XDG desktop portal.
  `scripts/package-portable.sh linux-arm64 target/release/prints-desktop`
  builds the portable archive from the finished executable.
- **Windows x64** · Build with Rust and the Visual Studio C++ build tools, or
  use the GNU Rust target and MinGW-w64. To package a native GNU cross-build,
  run `scripts/package-portable.sh windows-x64 target/x86_64-pc-windows-gnu/release/prints-desktop.exe`.
  Consult the release notes for testing and signing status.

## Hosted Prints

This preview explores local artwork. The connection screen links to upstream setup.
Oyster is Fungi's app platform. It runs Prints and authorises access to its
library, model catalogue and selected Computer. Darkroom supplies provider
adapters. A compatible installation and scoped credentials are required to
browse owner assets. Generation may incur provider and Cloudflare charges.
Exploring the offline workspace incurs no inference or Cloudflare usage.

See Fungi's [Prints and Workers AI guide](https://docs.fungi.computer/apps/prints)
for what it takes to set up your own backend. Upstream host, storage,
Computer and inference costs depend on the configuration.

The [read-only Oyster diagnostic](docs/oyster.md) documents the scoped connection,
supported methods and verification steps for developers.

Follow [the project issue](https://github.com/loftwah/app-composer/issues/1292)
for delivery evidence and integration status. See [artwork provenance](assets/README.md)
and [the licence](LICENSE).
