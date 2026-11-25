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
archive="prints-desktop-${version}-macos-arm64.zip"
minimum_os=14.0
identity="${PRINTS_CODESIGN_IDENTITY:-}"
notary_profile="${PRINTS_NOTARY_PROFILE:-}"
if [ -n "$identity" ] || [ -n "$notary_profile" ]; then
  [[ "$identity" == "Developer ID Application:"* && -n "$notary_profile" ]] || {
    printf '%s\n' 'Supply a Developer ID Application identity and a notary keychain profile together.' >&2
    exit 1
  }
  signing=developer-id
else
  signing=ad-hoc
fi

stage="$(mktemp -d "${TMPDIR:-/tmp}/prints-macos.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
app="$stage/Prints Desktop.app"
destination="dist/$version/Prints Desktop.app"
test ! -e "$destination" || {
  printf 'Remove or move the existing versioned bundle before repackaging: %s\n' "$destination" >&2
  exit 1
}

MACOSX_DEPLOYMENT_TARGET="$minimum_os" cargo build --locked --release
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "${CARGO_TARGET_DIR:-target}/release/prints-desktop" "$app/Contents/MacOS/prints-desktop"
install -m 644 LICENSE "$app/Contents/Resources/LICENSE"
install -m 644 assets/archive/README.md "$app/Contents/Resources/Creative-Archive.md"
mkdir -p "$app/Contents/Resources/licenses"
install -m 644 licenses/*.txt "$app/Contents/Resources/licenses/"

iconset="$stage"
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
printf 'version=%s\nsource=%s\nplatform=macos-arm64\nsigning=%s\n' "$version" "$revision" "$signing" > "$app/Contents/Resources/build.txt"

plutil -lint "$app/Contents/Info.plist"
if [ "$signing" = developer-id ]; then
  codesign --force --options runtime --timestamp --sign "$identity" "$app"
  ditto -c -k --sequesterRsrc --keepParent "$app" "$stage/notary.zip"
  xcrun notarytool submit "$stage/notary.zip" --keychain-profile "$notary_profile" --wait
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  spctl --assess --type execute "$app"
else
  codesign --force --sign - "$app"
fi
codesign --verify --strict "$app"
test "$(git rev-parse HEAD)" = "$revision"
test -z "$(git status --porcelain)"
ditto -c -k --sequesterRsrc --keepParent "$app" "$stage/$archive"
mkdir -p "dist/$version"
mv "$app" "$destination"
mv "$stage/$archive" "dist/$archive"
(
  cd dist
  shasum -a 256 "$archive" > "$archive.sha256"
)
printf 'Created %s and %s\n' "$destination" "dist/$archive"
