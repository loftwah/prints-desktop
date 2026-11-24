#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
test "$(uname -sm)" = "Darwin arm64" || {
  printf '%s\n' 'This packaging script targets Apple Silicon macOS.' >&2
  exit 1
}

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
test -z "$(git status --porcelain)" || {
  printf '%s\n' 'Commit the release inputs before packaging.' >&2
  exit 1
}
revision="$(git rev-parse HEAD)"
app="dist/Prints Desktop.app"
archive="prints-desktop-${version}-macos-arm64.zip"
minimum_os=14.0

MACOSX_DEPLOYMENT_TARGET="$minimum_os" cargo build --locked --release
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 target/release/prints-desktop "$app/Contents/MacOS/prints-desktop"
install -m 644 LICENSE "$app/Contents/Resources/LICENSE"
mkdir -p "$app/Contents/Resources/licenses"
install -m 644 licenses/*.txt "$app/Contents/Resources/licenses/"

iconset="$(mktemp -d "${TMPDIR:-/tmp}/prints-icon.XXXXXX")"
trap 'rm -rf "$iconset"' EXIT
mkdir "$iconset/Prints.iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/icon.png --out "$iconset/Prints.iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" assets/icon.png --out "$iconset/Prints.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset/Prints.iconset" -o "$app/Contents/Resources/Prints.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Prints Desktop</string>
  <key>CFBundleExecutable</key><string>prints-desktop</string>
  <key>CFBundleIconFile</key><string>Prints</string>
  <key>CFBundleIdentifier</key><string>com.loftwah.prints-desktop</string>
  <key>CFBundleName</key><string>Prints Desktop</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>$minimum_os</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
printf 'version=%s\nsource=%s\nplatform=macos-arm64\nsigning=ad-hoc\n' "$version" "$revision" > "$app/Contents/Resources/build.txt"

plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --strict "$app"
ditto -c -k --sequesterRsrc --keepParent "$app" "dist/$archive"
(
  cd dist
  shasum -a 256 "$archive" > "$archive.sha256"
)
printf 'Created %s and %s\n' "$app" "dist/$archive"
