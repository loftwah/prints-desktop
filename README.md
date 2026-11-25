# Prints Desktop

**Compare the artwork. Keep the original.**

Prints is a native workbench for image makers. Compare a cover and its release poster side by side, shortlist the versions worth keeping, and export the original file byte for byte. Open it offline to explore finished work from LoftwahFM, Fighter, and Bubbles, or bring your own images.

[Visit prints.loftwah.com](https://prints.loftwah.com/) · [Preview releases](https://github.com/loftwah/prints-desktop/releases) · [Contribute](CONTRIBUTING.md)

![The native Prints Desktop workspace. The Loftwah Creative Archive shows the Higher Than Before album cover, its contact sheet and original-export control.](docs/screenshots/creative-archive-v0.4.0.webp)

*Prints Desktop v0.4.0 on macOS, with published artwork from the [Loftwah Creative Archive](assets/archive/README.md).*

## Pick a starting point

| If you want to… | Start here |
| --- | --- |
| **See what the UI does** | Open the [creative archive in your browser](https://prints.loftwah.com/#archive). Follow the [native walkthrough](https://prints.loftwah.com/#walkthrough) to compare **Higher Than Before** and export an original. |
| **Work offline** | Download the preview and import a JPEG, PNG or WebP from your computer. The offline library needs no account, wallet, token, or model credits. |
| **Work with an agent** | Connect an approved client to your Oyster host’s `/mcp` endpoint with a Prints-scoped key. Start with the read-only `execute` example in the [Oyster guide](https://docs.fungi.computer/guides/call-an-app-from-your-own-server). |
| **Use Shiitake** | Open Shiitake in your project folder and give it the release art brief below. For your first tool call, use your own authorised Fungi Team or Oyster connection. [Shiitake terminal guide](https://docs.fungi.computer/guides/use-the-terminal) |
| **Build with AI** | Start with one brief and one target size. Create through web Prints, then inspect the resulting file in the desktop app. [Open Prints and choose a prompt](https://prints.loftwah.com/) |
| **Run the stack yourself** | Start with the free local Oyster trial. Add an authorised Workers AI model when you need generation and use AI Gateway when you need to observe supported requests. [Oyster trial](https://docs.fungi.computer/oyster-quickstart) · [Prints on Cloudflare](https://docs.fungi.computer/apps/prints) |

### Try a real brief

Use this in your agent or image tool. In the desktop app, **Prompt draft** is a local place to work on the text.

```text
Make a 4:5 release poster for an imaginary indie game called Night Shift.
Show one orange desk lamp illuminating a tiny coastal town built from paper.
Use warm orange, deep ink and off-white, with a single point of focus.
Set only the words NIGHT SHIFT. Keep the letters readable at 300 pixels wide.
Return one version. Keep the original image and save the prompt with it.
```

If you already have a scoped Oyster connection, this is a useful first agent request.

```text
Use the approved Oyster connection to list Prints outputs. Tell me which
images are portrait and which are square, using the metadata already returned.
Keep this read-only. Ask me to select an output before exporting anything.
```

The desktop app reads compatible Oyster libraries and hands creation off to signed-in web Prints. Hosted generation requires model access and funding. [Connection and permission details](docs/oyster.md) · [Native creation status](https://github.com/loftwah/prints-desktop/issues/6)

## Get the preview

Get the archives and SHA-256 checksums from the [v0.4.1 release](https://github.com/loftwah/prints-desktop/releases/tag/v0.4.1).

| Platform | Run it | Verified |
| --- | --- | --- |
| **Apple Silicon Mac** · macOS 14+ | Extract the ZIP and open `Prints Desktop.app`. [Gatekeeper guidance](https://support.apple.com/en-au/102445) | The extracted app opened on Apple Silicon macOS 26.6.2 and exported the bundled Fighter PNG byte for byte through the native Save dialog. Ad-hoc signed and unnotarised. |
| **Linux** · x64 or ARM64 | Extract the tarball and run `prints-desktop` in a graphical session. [Runtime prerequisites](CONTRIBUTING.md#platform-setup) | Both packages opened in Debian graphical test sessions. Find and Escape worked, and the x64 session used OrbStack CPU translation. |
| **Windows** · x64 | Extract the ZIP and run `prints-desktop.exe`. | Unsigned x64 cross-build. Archive and embedded source checks passed. A Windows desktop launch is unverified. |

[Windows and Mac distribution status](https://github.com/loftwah/prints-desktop/issues/7) · [Public download counts](https://prints.loftwah.com/downloads)

## Build or help

Install [Rust with rustup](https://rustup.rs/), check the [platform setup](CONTRIBUTING.md#platform-setup), then run:

```sh
git clone https://github.com/loftwah/prints-desktop.git
cd prints-desktop
cargo run --locked
```

The app uses Rust and [Rune](https://github.com/anomalyco/rune), pinned in `Cargo.toml`. Prints runs independently. App Composer lists the project for discovery. Composer operators can enable the optional [Prints admin plugin](https://github.com/loftwah/app-composer/blob/main/docs/prints-integration.md).

**First contribution:** Try comparison or local import with a public image and tell us the exact point where you hesitate. Include your platform, release, and the steps you took. A reproducible observation or a small docs fix is useful on its own.

[Contribution guide](CONTRIBUTING.md) · [Open an issue](https://github.com/loftwah/prints-desktop/issues/new/choose) · [Product direction](PRODUCT.md) · [App Composer project](https://composer.loftwah.com/prints-desktop) · [MIT licence](LICENSE)
