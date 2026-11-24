#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
scratch="$(mktemp -d "${TMPDIR:-/tmp}/prints-check.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
cargo build --locked
python3 - "$scratch" <<'PY'
import json
from pathlib import Path
import sys

scratch = Path(sys.argv[1])
steps = [
    (0.2, "tap", {"key": "print-2"}),
    (0.4, "tap", {"key": "favourite"}),
    (0.7, "tap", {"key": "favourites"}),
    (0.9, "tap", {"key": "preview"}),
    (1.2, "key", {"name": "Escape"}),
    (1.4, "tap", {"key": "export"}),
    (1.7, "file-dialog", {"path": str(scratch / "export.jpg")}),
    (2.0, "tap", {"key": "use-prompt"}),
    (2.2, "tap", {"key": "save-draft"}),
    (2.5, "tap", {"key": "imports"}),
    (2.7, "tap", {"key": "import"}),
    (3.0, "file-dialog", {"path": str(Path("assets/blue-hour.jpg").resolve())}),
    (3.4, "tap", {"key": "theme"}),
]
(scratch / "steps.json").write_text(json.dumps([
    {"t": t, "type": kind, **event} for t, kind, event in steps
]))
PY
binary=target/debug/prints-desktop
if [ -f "$binary.exe" ]; then binary="$binary.exe"; fi
"$binary" --script "$scratch/steps.json" --tree "$scratch/tree.json" --at 3.8 --size 1010x690 --settle
cmp assets/salt.jpg "$scratch/export.jpg"
python3 - "$scratch/tree.json" <<'PY'
from pathlib import Path
import sys
tree = Path(sys.argv[1]).read_text()
for name in ("Local imports", "blue-hour", "Dark appearance", "Export original"):
    assert name in tree, f"Missing from final workspace: {name}"
PY
printf '%s\n' 'Workspace replay passed. The exported JPEG exactly matches the bundled original.'
