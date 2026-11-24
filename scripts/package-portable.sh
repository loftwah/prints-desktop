#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
platform="${1:?Choose linux-arm64 or windows-x64}"
binary="${2:?Provide the release binary for the named platform}"
binary="$(realpath "$binary")"
case "$platform" in
  linux-arm64)
    file -b "$binary" | grep -q 'ELF.*ARM aarch64'
    executable="prints-desktop"
    extension="tar.gz"
    ;;
  windows-x64)
    file -b "$binary" | grep -q 'PE32+.*x86-64'
    executable="Prints Desktop.exe"
    extension="zip"
    ;;
  *)
    printf 'Unsupported portable target: %s\n' "$platform" >&2
    exit 1
    ;;
esac

test -z "$(git status --porcelain)" || {
  printf '%s\n' 'Commit the release inputs before packaging.' >&2
  exit 1
}
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
revision="$(git rev-parse HEAD)"
name="prints-desktop-${version}-${platform}"
archive="$name.$extension"

stage="$(mktemp -d "${TMPDIR:-/tmp}/prints-portable.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name/licenses" dist
install -m 755 "$binary" "$stage/$name/$executable"
install -m 644 LICENSE README.md "$stage/$name/"
install -m 644 licenses/*.txt "$stage/$name/licenses/"
printf 'version=%s\nsource=%s\nplatform=%s\n' "$version" "$revision" "$platform" > "$stage/$name/build.txt"

if [ "$extension" = zip ]; then
  archive_path="$PWD/dist/$archive"
  (cd "$stage" && zip -q -r "$archive_path" "$name")
else
  tar -czf "dist/$archive" -C "$stage" "$name"
fi
(
  cd dist
  shasum -a 256 "$archive" > "$archive.sha256"
)
printf 'Created %s\n' "dist/$archive"
