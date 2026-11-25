# Prints Desktop

Open **Afterlight**, six studies in the light that lingers. Browse the contact
sheet, see a print at full size, keep favourites, and export the included
JPEGs. Bring an image from your own machine or use a sample's prompt as a
starting point for an editable, local draft. Sample favourites, saved drafts,
local imports and their favourites reopen on this device.

Local imports keep links to up to 12 original files. The first time you open
Local imports or Favourites, Prints reads those files from their saved
locations. If a file is unavailable, choose **Refresh original** to reopen its
saved location or **Locate original…** to choose its new location. **Remove
from library** clears the link and its favourite. The original stays on disk.

Prints Desktop is a standalone native Rust application built with
[Anomaly's Rune](https://github.com/anomalyco/rune). Its offline workspace
needs no App Composer account, server or inference credits. It is an
independent Loftwah project for the [Fungi](https://docs.fungi.computer/)
ecosystem.

## Download

Open the [preview release](https://github.com/loftwah/prints-desktop/releases/tag/v0.3.0)
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
python3 scripts/check-connected-workspace.py
python3 scripts/check-local-library.py
cargo run --locked -- --snap prints.png --size 1360x840 --settle
scripts/package-macos.sh
```

To check the local library across a real restart without touching your own
preferences, point Rune at an isolated directory for both launches:

```sh
dir=$(mktemp -d)
RUNE_PREFS_DIR="$dir" cargo run --locked   # import an image, favourite it, quit
RUNE_PREFS_DIR="$dir" cargo run --locked   # Local imports and its favourite return
```

The workspace replays require Bash and Python 3. They use in-memory preferences
or an isolated test file. The connected replay uses a fictional loopback
Oyster. Packaging on Apple Silicon writes the app under `dist/<version>/`
and the ZIP and SHA-256 checksum to `dist/`. The bundle records its release
version and Git revision in `Contents/Resources/build.txt`.

## Platform builds

- **Apple Silicon macOS** · `scripts/package-macos.sh` creates the app and archive.
  It targets macOS 14 or later. The release notes identify the tested OS.
  A public Developer ID build uses `PRINTS_CODESIGN_IDENTITY` and
  `PRINTS_NOTARY_PROFILE` with an installed Developer ID Application
  certificate and a preconfigured notary profile in Keychain. The packaging
  script submits, staples and assesses that build before writing its archive.
- **Linux ARM64 and x64** · Build with `cargo build --locked --release` using Rust 1.99.1.
  On Debian, install `libdbus-1-dev`, `libxkbcommon-dev` and `libwayland-dev` to compile.
  A graphical session needs the X11 or Wayland client libraries and working
  graphics drivers. Native Open and Save use the XDG desktop portal.
  Run `scripts/package-portable.sh linux-arm64 target/release/prints-desktop`
  for ARM64 or use `linux-x64` for x64.
- **Windows x64** · Build with Rust and the Visual Studio C++ build tools, or
  use the GNU Rust target and MinGW-w64. To package a native GNU cross-build,
  run `scripts/package-portable.sh windows-x64 target/x86_64-pc-windows-gnu/release/prints-desktop.exe`.
  Consult the release notes for testing and signing status.

## Hosted Prints

From **Connect Prints**, enter your Oyster origin, Prints installation ID and
named App key scoped to that installation. Browse indexed files, inspect media
and prompt metadata, view supported image previews and export originals. Older compatible installations show the root library and expose creation and
folder browsing through web Prints. Choose whether to keep the key in the
system credential store. A working Secret
Service is required to remember a connection on Linux.

The connected preview opens the signed-in web Prints app for creation. Oyster
identifies named App keys as Team keys and the supported Prints release declines
generation from that actor type. Native generation requires a supported
scoped member or agent sign-in.

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
