#!/usr/bin/env python3
"""Replay import, restart, relocation and removal with isolated preferences."""
import json
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "debug" / ("prints-desktop.exe" if sys.platform == "win32" else "prints-desktop")
ORIGINAL = (ROOT / "assets/blue-hour.jpg").read_bytes()


def replay(run, actions):
    steps = run / "steps.json"
    tree = run / "tree.json"
    steps.write_text(json.dumps([
        {"t": time, "drop_files": details} if event == "drop_files"
        else {"t": time, "type": event, **details}
        for time, event, details in actions
    ]))
    subprocess.run(
        [str(BINARY), "--script", str(steps), "--tree", str(tree), "--at", str(actions[-1][0] + 0.5),
         "--size", "1010x690", "--settle", "--realtime"],
        check=True, timeout=15,
        env={**os.environ, "PRINTS_REPLAY_PREFS": str(run / "prefs.json")},
    )
    return json.loads(tree.read_text())


def text_in(tree, expected):
    assert expected in json.dumps(tree, ensure_ascii=False), expected


with TemporaryDirectory(prefix="prints-local-library-") as temporary:
    run = Path(temporary)
    original = run / "Aurora.jpg"
    original.write_bytes(ORIGINAL)
    text_in(replay(run, [
        (0.2, "tap", {"key": "import"}),
        (0.5, "file-dialog", {"path": str(original)}),
        (0.9, "tap", {"key": "favourite"}),
    ]), "Remove favourite")

    destination = run / "export.jpg"
    text_in(replay(run, [
        (0.2, "tap", {"key": "favourites"}),
        (0.5, "tap", {"key": "export"}),
        (0.8, "file-dialog", {"path": str(destination)}),
    ]), "Select Aurora")
    assert destination.read_bytes() == ORIGINAL

    moved = run / "Evening.jpg"
    original.rename(moved)
    missing = replay(run, [(0.2, "tap", {"key": "imports"})])
    text_in(missing, "Locate original")
    text_in(missing, "Remove favourite")

    relocated = replay(run, [
        (0.2, "tap", {"key": "imports"}),
        (0.4, "tap", {"key": "search-input"}),
        (0.6, "type", {"text": "aurora"}),
        (0.8, "tap", {"key": "locate-original"}),
        (1.0, "file-dialog", {"path": str(moved)}),
    ])
    text_in(relocated, "Original located.")
    assert "Open full view of Evening" not in json.dumps(relocated)
    assert "Export original" not in json.dumps(relocated)

    text_in(replay(run, [
        (0.2, "tap", {"key": "imports"}),
        (0.5, "tap", {"key": "remove-local"}),
    ]), "Removed from library.")
    assert moved.read_bytes() == ORIGINAL
    no_items = replay(run, [(0.2, "tap", {"key": "favourites"})])
    assert "Select Evening" not in json.dumps(no_items)

    images = [run / f"Batch {i:02}.jpg" for i in range(1, 14)]
    for image in images:
        image.write_bytes(ORIGINAL)
    unsupported = run / "notes.txt"
    unsupported.write_text("Not an image")
    paths = [unsupported, images[0], images[0], *images[1:]]
    batch = replay(run, [
        (0.2, "drop_files", {"at": [390, 280], "paths": [str(path) for path in paths]}),
    ])
    text_in(batch, "Linked 12 of 15 files. Library limit: 12.")
    persisted = replay(run, [(0.2, "tap", {"key": "imports"})])
    for image in images[:12]:
        text_in(persisted, f"Select {image.stem}")
        assert image.read_bytes() == ORIGINAL
    assert "Select Batch 13" not in json.dumps(persisted)

    compared = replay(run, [
        (0.2, "tap", {"key": "imports"}),
        (0.4, "tap", {"key": "choose-comparison"}),
        (0.6, "tap", {"key": "print-12"}),
        (0.8, "tap", {"key": "compare-local"}),
    ])
    for expected in ("local-comparison", "Batch 01.jpg", "Batch 02.jpg"):
        text_in(compared, expected)

print("Local imports, comparisons, restart, relocation and original preservation passed.")
