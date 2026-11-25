#!/usr/bin/env python3
"""Replay import, restart, relocation and removal with isolated preferences."""
import json
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug" / ("prints-desktop.exe" if sys.platform == "win32" else "prints-desktop")
ORIGINAL = (ROOT / "assets/blue-hour.jpg").read_bytes()


def replay(run, actions):
    steps = run / "steps.json"
    tree = run / "tree.json"
    steps.write_text(json.dumps([
        {"t": time, "type": event, **details}
        for time, event, details in actions
    ]))
    subprocess.run(
        [str(BINARY), "--script", str(steps), "--tree", str(tree), "--at", str(actions[-1][0] + 0.5),
         "--size", "1010x690", "--settle"],
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

    moved = run / "Aurora moved.jpg"
    original.rename(moved)
    missing = replay(run, [(0.2, "tap", {"key": "imports"})])
    text_in(missing, "Locate original")
    text_in(missing, "Remove favourite")

    text_in(replay(run, [
        (0.2, "tap", {"key": "imports"}),
        (0.5, "tap", {"key": "locate-original"}),
        (0.8, "file-dialog", {"path": str(moved)}),
    ]), "Original located.")

    text_in(replay(run, [
        (0.2, "tap", {"key": "imports"}),
        (0.5, "tap", {"key": "remove-local"}),
    ]), "Removed from library.")
    assert moved.read_bytes() == ORIGINAL
    no_items = replay(run, [(0.2, "tap", {"key": "favourites"})])
    assert "Select Aurora" not in json.dumps(no_items)

print("Local library replay passed across restarts. Relocation and removal preserved the original.")
