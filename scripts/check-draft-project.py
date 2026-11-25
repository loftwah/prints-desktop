#!/usr/bin/env python3
"""Resume a saved brief, locate its moved image and export the original."""
import json
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "debug" / ("prints-desktop.exe" if sys.platform == "win32" else "prints-desktop")
ORIGINAL = (ROOT / "assets/blue-hour.jpg").read_bytes()


def tap(time, key):
    return {"t": time, "type": "tap", "key": key}


def dialog(time, path):
    return {"t": time, "type": "file-dialog", "path": str(path)}


def replay(run, steps):
    actions = run / "actions.json"
    tree = run / "tree.json"
    actions.write_text(json.dumps(steps))
    subprocess.run(
        [str(BINARY), "--script", str(actions), "--tree", str(tree), "--at",
         str(steps[-1]["t"] + 0.55), "--size", "940x660", "--settle", "--realtime"],
        check=True, timeout=15,
        env={**os.environ, "PRINTS_REPLAY_PREFS": str(run / "prefs.json")},
    )
    return tree.read_text()


with TemporaryDirectory(prefix="prints-draft-project-") as temporary:
    run = Path(temporary).resolve()
    original = run / "First study.jpg"
    original.write_bytes(ORIGINAL)
    prefs = run / "prefs.json"
    saved_draft = "Compare the two releases before choosing a poster."
    prefs.write_text(json.dumps({
        "draft": saved_draft,
        "linked-originals": [{"path": str(original), "favourite": True}],
    }))

    draft = replay(run, [
        tap(0.2, "keep-with-draft"),
        tap(0.4, "imports"),
        tap(0.7, "keep-with-draft"),
        tap(1.0, "draft"),
    ])
    for marker in ["Draft shortlist", "Higher Than Before", "First study", saved_draft]:
        assert marker in draft, marker
    stored = json.loads(prefs.read_text())
    assert stored["linked-originals"] == [
        {"path": str(original), "favourite": True, "in_draft": True},
    ]

    moved = run / "Selected study.jpg"
    original.rename(moved)
    recovered = replay(run, [
        tap(0.2, "draft"),
        tap(0.5, "draft-open-11"),
        tap(0.9, "locate-original"),
        dialog(1.2, moved),
        tap(1.7, "draft"),
    ])
    assert "Selected study" in recovered
    stored = json.loads(prefs.read_text())
    assert stored["linked-originals"] == [
        {"path": str(moved), "favourite": True, "in_draft": True},
    ]

    destination = run / "Selected export.jpg"
    exported = replay(run, [
        tap(0.2, "draft"),
        tap(0.5, "draft-open-11"),
        tap(0.9, "export"),
        dialog(1.2, destination),
    ])
    assert "Original exported." in exported
    assert destination.read_bytes() == ORIGINAL

    removed = replay(run, [tap(0.2, "draft"), tap(0.5, "draft-remove-11")])
    assert "Removed from draft." in removed
    assert "Open Selected study" not in removed
    stored = json.loads(prefs.read_text())
    assert stored["linked-originals"] == [
        {"path": str(moved), "favourite": True},
    ]
    assert stored["draft"] == saved_draft
    assert moved.read_bytes() == ORIGINAL

print("Saved brief, shortlist, relocation, restart and byte-exact export passed.")
