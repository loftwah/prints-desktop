#!/usr/bin/env python3
"""Recover an unfinished brief, locate a moved reference and export exact bytes."""
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


def replay(run, steps, *, wait=0.55):
    actions = run.parent / "actions.json"
    tree = run.parent / "tree.json"
    actions.write_text(json.dumps(steps))
    subprocess.run(
        [str(BINARY), "--script", str(actions), "--tree", str(tree), "--at",
         str(steps[-1]["t"] + wait), "--size", "940x660", "--settle", "--realtime"],
        check=True, timeout=15,
        env={**os.environ, "PRINTS_REPLAY_PREFS": str(run / "prefs.json")},
    )
    return tree.read_text()


with TemporaryDirectory(prefix="prints-draft-project-") as temporary:
    run = Path(temporary).resolve() / "profile"
    run.mkdir()
    original = run / "First study.jpg"
    original.write_bytes(ORIGINAL)
    prefs = run / "prefs.json"
    working = run / "prefs.draft-workspace.json"
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
    def workspace():
        return json.loads(working.read_text())["workspace"]

    assert workspace()["library"] == [{"path": str(original), "favourite": True}]
    assert workspace()["shortlist"] == [
        {"Bundled": "higher-than-before.jpg"}, {"Linked": str(original)},
    ]

    new_brief = saved_draft + " Keep the square version for the album tile."
    edited = replay(run, [
        tap(0.2, "draft"), tap(0.5, "draft-input"),
        {"t": 0.6, "type": "key", "name": "a", "cmd": True},
        {"t": 0.8, "type": "paste", "text": new_brief},
    ])
    assert "Draft saved on this device" in edited
    assert workspace()["text"] == new_brief

    checkpoint = working.read_bytes()
    interrupted = replay(run, [
        tap(0.2, "draft"), tap(0.5, "draft-input"),
        {"t": 0.65, "type": "key", "name": "a", "cmd": True},
        {"t": 0.8, "type": "paste", "text": "An edit before the save deadline."},
    ], wait=0.1)
    assert "Saving draft" in interrupted
    assert working.read_bytes() == checkpoint

    if hasattr(os, "geteuid") and os.geteuid() != 0:
        original_mode = run.stat().st_mode
        try:
            run.chmod(0o500)
            failed = replay(run, [
                tap(0.2, "draft"), tap(0.5, "draft-input"),
                {"t": 0.65, "type": "key", "name": "a", "cmd": True},
                {"t": 0.8, "type": "paste", "text": "The storage failure fixture."},
            ])
            assert "Couldn’t save the draft" in failed
            assert "The storage failure fixture." in failed
            assert working.read_bytes() == checkpoint
        finally:
            run.chmod(original_mode)

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
    assert new_brief in recovered
    assert workspace()["library"] == [{"path": str(moved), "favourite": True}]
    assert workspace()["shortlist"][-1] == {"Linked": str(moved)}

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
    assert workspace()["library"] == [{"path": str(moved), "favourite": True}]
    assert workspace()["shortlist"] == [{"Bundled": "higher-than-before.jpg"}]
    assert workspace()["text"] == new_brief
    cleared = replay(run, [tap(0.2, "draft"), tap(0.5, "clear-draft")])
    assert "Undo clear" in cleared and workspace()["text"] == ""
    restored = replay(run, [tap(0.2, "draft"), tap(0.5, "restore-cleared-draft")])
    assert new_brief in restored and workspace()["text"] == new_brief
    assert moved.read_bytes() == ORIGINAL

print("Draft autosave, restart, recovery, relocation and byte-exact export passed.")
