# Contributing to Prints Desktop

Start with a real action in the app. Find a print, compare its formats, import a public image, or save an original. If a step causes friction, a short report with your platform and steps is enough. Newcomers can fix a guide, improve a keyboard path, or add a reproduction as their first contribution.

- [Report a bug or documentation problem](https://github.com/loftwah/prints-desktop/issues/new/choose)
- [See the current product benchmark](https://github.com/loftwah/prints-desktop/issues/4)
- [See the platform verification work](https://github.com/loftwah/prints-desktop/issues/7)
- [Read the product direction](PRODUCT.md)

## Platform setup

Install [rustup](https://rustup.rs/). The checkout selects the toolchain in `rust-toolchain.toml`. Rune and its platform patches are pinned to a revision in `Cargo.toml`.

| Platform | Prerequisites |
| --- | --- |
| Apple Silicon macOS | Xcode command-line tools. The packaged app targets macOS 14 or later. |
| Debian Linux | Install `libdbus-1-dev`, `libxkbcommon-dev`, and `libwayland-dev` to build. A minimal X11 session also needs `libx11-xcb1`, `libxcursor1`, `libxi6`, and `libxkbcommon-x11-0`. Open and Save use the XDG desktop portal. |
| Windows x64 | Visual Studio C++ Build Tools for the MSVC target, or MinGW-w64 for the GNU target. See the release for actual signing and runtime coverage. |

From the repository root, use `cargo run --locked`. On Linux, optional app-menu integration uses the archive’s desktop entry and icon. Put the binary on your PATH, the desktop entry in `~/.local/share/applications`, and its PNG in `~/.local/share/icons/hicolor/512x512/apps`.

## Prove the action you changed

Use the smallest check that exercises the affected behaviour. Rust’s unit checks and Rune’s scripted UI complement a real launch.

| Change | Run |
| --- | --- |
| Rust code | `cargo fmt --all -- --check` and `cargo test --locked` |
| Archive selection, Find, or comparison | `python3 scripts/check-archive.py` |
| Workspace navigation and Afterlight | `scripts/check-workspace.sh` |
| Local originals and saved links | `python3 scripts/check-local-library.py` |
| Oyster connection and library | `python3 scripts/check-connected-workspace.py` |

The scripted checks use Bash or Python 3 and isolate preferences. The connected replay uses fictional loopback data. For a view change, capture the actual result through Rune:

```sh
cargo run --locked -- --snap prints.png --size 1360x840 --settle
```

Use public or fictional inputs in reports, screenshots and recordings. Keep credentials, private artwork, and local paths out of issues and pull requests. Describe what changed, which journey you exercised, the operating system, and the checks you ran. Keep the original sources and attribution with any new archive art.

## Packaging

- **Mac** · Run `scripts/package-macos.sh`. Outputs go to `dist/` and record the Git revision in the bundle. Developer ID signing and notarisation use `PRINTS_CODESIGN_IDENTITY` and `PRINTS_NOTARY_PROFILE` with an installed certificate and configured Keychain profile.
- **Linux** · Run `cargo build --locked --release`, then `scripts/package-portable.sh linux-arm64 target/release/prints-desktop` or use `linux-x64` for that architecture.
- **Windows GNU** · Build for `x86_64-pc-windows-gnu`, then run `scripts/package-portable.sh windows-x64 target/x86_64-pc-windows-gnu/release/prints-desktop.exe`.

A successful build proves that target compiled. Record any real or virtual desktop launches, signing, and what remains unverified in the release notes. Compare the final archive against its SHA-256 file before using it.

[Desktop workspace design](docs/native-workspace.md) · [Oyster transport and verification](docs/oyster.md) · [Artwork provenance](assets/README.md)
