#!/usr/bin/env python3
"""Replay archive search, menu, comparison, saved favourite and byte-exact export."""
import json
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "debug" / (
    "prints-desktop.exe" if sys.platform == "win32" else "prints-desktop"
)


def replay(run, actions, present, absent=()):
    script = run / "steps.json"
    script.write_text(json.dumps([
        {"t": 0.2 + index * 0.25, **action} for index, action in enumerate(actions)
    ]))
    tree = run / "tree.json"
    subprocess.run(
        [str(BINARY), "--script", str(script), "--tree", str(tree),
         "--at", str(0.4 + len(actions) * 0.25), "--size", "940x660", "--settle", "--realtime"],
        check=True, timeout=15,
        env={**os.environ, "PRINTS_REPLAY_PREFS": str(run / "prefs.json")},
    )
    result = json.dumps(json.loads(tree.read_text()), ensure_ascii=False)
    for text in present:
        assert text in result, f"Missing in workspace: {text}"
    for text in absent:
        assert text not in result, f"Unexpected in workspace: {text}"


def key(name, **modifiers):
    return {"type": "key", "name": name, **modifiers}


def tap(name):
    return {"type": "tap", "key": name}


with TemporaryDirectory(prefix="prints-archive-") as temporary:
    run = Path(temporary)
    replay(run, [tap("compare-editions")],
           ("campaign-comparison", "Album cover", "Release poster"))
    replay(run, [key("f", cmd=True), key("b"), key("e"), key("l"), key("l")],
           ("1 print found", "Before the Bell", "FLUX.2 Klein"), ("Select Release Poster",))
    replay(run, [tap("search-input"), key("z"), key("z")], ("No prints",))
    replay(run, [tap("search-input"), key("z"), key("Escape")],
           ("Select Higher Than Before", "Select One Shot"), ("No prints",))
    replay(run, [tap("favourite")], ("Remove favourite",))
    exported = run / "higher-than-before.jpg"
    replay(run, [tap("favourites"), tap("export"), {"type": "file-dialog", "path": str(exported)}],
           ("Select Higher Than Before", "Remove favourite"))
    assert exported.read_bytes() == (ROOT / "assets/archive/higher-than-before.jpg").read_bytes()

print("Archive replay passed. Find, comparison and saved favourite exercised, export matches the published original.")
