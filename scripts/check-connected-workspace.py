#!/usr/bin/env python3
"""Exercise the native Prints library against a loopback-only fictional Oyster."""
import argparse
import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "debug" / ("prints-desktop.exe" if sys.platform == "win32" else "prints-desktop")
ASSET = (ROOT / "assets/horizon.jpg").read_bytes()
KEY = "oyk_fictional_local_app_key"
CALLS = []


def output(computer="fictional-computer"):
    return {
        "path": "/library/Studies/horizon.jpg",
        "print": {
            "printId": "print_fixture",
            "computerId": computer,
            "kind": "image",
            "model": "fictional-image-model",
            "format": "jpg",
            "bytes": len(ASSET),
            "width": 1280,
            "height": 720,
            "createdAt": 1730000000000,
        },
        "characters": ["Garden keeper"],
        "style": "Field notes",
        "typed": {"prompt": "A fictional landscape from a local fixture.", "references": 1},
    }


class OysterFixture(BaseHTTPRequestHandler):
    revoke = False
    legacy = False
    transient = False
    folder_error = False
    keep_local = False

    def log_message(self, *_):
        pass

    def do_POST(self):
        if self.headers.get("Authorization") != f"Bearer {KEY}":
            self.reply(401, {"error": "not_granted"})
            return
        operation = self.path.removeprefix("/apps/prints/methods/")
        if operation == self.path:
            self.reply(404, {"error": "not_found"})
            return
        arguments = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        CALLS.append((operation, arguments))
        if operation == "listOutputs":
            if "recursive" in arguments and (
                self.legacy or (self.transient and len(CALLS) == 1)
            ):
                self.reply(502, {"error": "App execution failed"})
            elif self.revoke and arguments.get("folder") == "Studies":
                self.reply(200, {"error": "revoked"})
            elif self.folder_error and arguments.get("folder") == "Studies":
                self.reply(502, {"error": "App execution failed"})
            else:
                self.reply(200, {"items": [output()], "next": None})
        elif operation == "listModels":
            if self.keep_local:
                time.sleep(0.65)
            self.reply(200, {"models": [{"id": "fictional-image-model", "label": "Fixture model", "kind": "image"}]})
        elif operation == "listFolders":
            if self.legacy:
                self.reply(403, {"error": "not_declared"})
            else:
                self.reply(200, {"folders": [{"name": "Studies", "folder": "Studies"}], "next": None})
        elif operation == "getOutput":
            self.reply(200, output())
        elif operation == "openOutput":
            offset = arguments["byteOffset"]
            chunk = ASSET[offset : offset + arguments["byteLength"]]
            self.reply(200, {
                "path": output()["path"],
                "printId": "print_fixture",
                "byteOffset": offset,
                "content": base64.b64encode(chunk).decode(),
                "encoding": "base64",
                "truncated": bool(chunk),
            })
        else:
            self.reply(404, {"error": "not_found"})

    def reply(self, status, body):
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def has_key(node, key):
    if isinstance(node, dict):
        return node.get("key") == key or any(has_key(value, key) for value in node.values())
    if isinstance(node, list):
        return any(has_key(value, key) for value in node)
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", nargs="?", type=Path)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--revoked", action="store_true")
    modes.add_argument("--legacy", action="store_true")
    modes.add_argument("--transient", action="store_true")
    modes.add_argument("--folder-error", action="store_true")
    modes.add_argument("--keep-local", action="store_true")
    options = parser.parse_args()
    OysterFixture.revoke = options.revoked
    OysterFixture.legacy = options.legacy
    OysterFixture.transient = options.transient
    OysterFixture.folder_error = options.folder_error
    OysterFixture.keep_local = options.keep_local
    with tempfile.TemporaryDirectory(prefix="prints-connection-proof-") as temporary:
        run = Path(temporary)
        with ThreadingHTTPServer(("127.0.0.1", 0), OysterFixture) as server:
            threading.Thread(target=server.serve_forever, daemon=True).start()
            url = f"http://127.0.0.1:{server.server_port}"
            actions = [
                (0.15, "tap", {"key": "connection"}),
                (0.35, "tap", {"key": "oyster-origin"}),
                (0.50, "type", {"text": url}),
                (1.50, "tap", {"key": "prints-key"}),
                (1.65, "type", {"text": KEY}),
                (2.75, "tap", {"key": "connect-hosted"}),
                (3.40, "tap", {"key": "browse-folders"}),
                (3.90, "tap", {"key": "folder-0"}),
                (4.40, "tap", {"key": "hosted-preview"}),
                (5.15, "tap", {"key": "hosted-save"}),
                (5.60, "tap", {"key": "browse-folders"}),
                (5.90, "file-dialog", {"path": str(run / "export.jpg")}),
            ]
            if options.revoked or options.folder_error:
                actions = [entry for entry in actions if entry[0] <= 3.90]
            elif options.keep_local:
                actions = [entry for entry in actions if entry[0] <= 2.75]
                actions.append((3.0, "tap", {"key": "afterlight"}))
            elif options.legacy:
                actions = [entry for entry in actions if entry[0] not in (3.40, 3.90, 5.60)]
            steps = run / "steps.json"
            steps.write_text(json.dumps([{"t": t, "type": kind, **options} for t, kind, options in actions]))
            tree = run / "tree.json"
            finish = "4.6" if (options.revoked or options.folder_error or options.keep_local) else "6.45"
            args = [str(BINARY), "--script", str(steps), "--tree", str(tree), "--at", finish,
                    "--size", "1100x740", "--settle", "--realtime"]
            if options.snapshot:
                args.extend(["--snap", str(options.snapshot.resolve()), "--scale", "1"])
            subprocess.run(args, cwd=ROOT, check=True, timeout=30)
            server.shutdown()

        contents = tree.read_text()
        nodes = json.loads(contents)
        assert KEY not in contents
        if options.revoked:
            assert "Prints refused listOutputs: revoked" in contents
            assert not has_key(nodes, "hosted-item-0")
            assert "A fictional landscape" not in contents
            assert all(operation != "openOutput" for operation, _ in CALLS)
            print("Revocation replay passed. The native workspace discarded the loaded library.")
            return
        if options.folder_error:
            assert "Showing the previous view." in contents
            assert has_key(nodes, "hosted-item-0")
            assert "A fictional landscape" in contents
            assert ("listOutputs", {"recursive": True, "limit": 24, "folder": "Studies"}) in CALLS
            print("Folder failure replay passed. The prior library and selection remained visible.")
            return
        if options.keep_local:
            assert "Six studies in the light that lingers." in contents
            assert "Prints connected" in contents
            assert not has_key(nodes, "hosted-item-0")
            print("Background connection replay passed. The local workspace kept focus.")
            return
        assert (run / "export.jpg").read_bytes() == ASSET
        for expected in ("Prints library", "Fixture model", "Style", "Garden keeper", "Original exported"):
            assert expected in contents, expected
        if options.legacy or options.transient:
            assert CALLS[:3] == [
                ("listOutputs", {"recursive": True, "limit": 24}),
                ("listOutputs", {"limit": 24}),
                ("listFolders", {"limit": 1}),
            ]
        if not options.legacy:
            expected_folder_calls = 2 if options.transient else 1
            assert sum(operation == "listFolders" for operation, _ in CALLS) == expected_folder_calls
        if options.legacy:
            assert "This installation exposes its root library" in contents
            assert 'browse-folders' not in contents
        else:
            assert ("listOutputs", {"recursive": True, "limit": 24, "folder": "Studies"}) in CALLS
        assert sum(operation == "getOutput" for operation, _ in CALLS) == 4
        assert any(operation == "openOutput" and request["byteOffset"] > 0 for operation, request in CALLS)
        print("Connected workspace replay passed. Exact original bytes exported from the fictional Oyster fixture.")


if __name__ == "__main__":
    main()
