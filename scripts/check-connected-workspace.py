#!/usr/bin/env python3
"""Exercise the native Prints library against a loopback-only fictional Oyster."""
import argparse
import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug" / ("prints-desktop.exe" if sys.platform == "win32" else "prints-desktop")
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
            if self.revoke and arguments.get("folder") == "Studies":
                self.reply(200, {"error": "revoked"})
            else:
                self.reply(200, {"items": [output()], "next": None})
        elif operation == "listModels":
            self.reply(200, {"models": [{"id": "fictional-image-model", "label": "Fixture model", "kind": "image"}]})
        elif operation == "listFolders":
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", nargs="?", type=Path)
    parser.add_argument("--revoked", action="store_true")
    options = parser.parse_args()
    OysterFixture.revoke = options.revoked
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
                (5.90, "file-dialog", {"path": str(run / "export.jpg")}),
            ]
            if options.revoked:
                actions = [entry for entry in actions if entry[0] <= 3.90]
            steps = run / "steps.json"
            steps.write_text(json.dumps([{"t": t, "type": kind, **options} for t, kind, options in actions]))
            tree = run / "tree.json"
            args = [str(BINARY), "--script", str(steps), "--tree", str(tree), "--at", "4.6" if options.revoked else "6.45",
                    "--size", "1100x740", "--settle", "--realtime"]
            if options.snapshot:
                args.extend(["--snap", str(options.snapshot.resolve()), "--scale", "1"])
            subprocess.run(args, cwd=ROOT, check=True, timeout=30)
            server.shutdown()

        contents = tree.read_text()
        assert KEY not in contents
        if options.revoked:
            assert "Prints refused listOutputs: revoked" in contents
            assert '"key":"hosted-item-0"' not in contents
            assert "A fictional landscape" not in contents
            assert all(operation != "openOutput" for operation, _ in CALLS)
            print("Revocation replay passed. The native workspace discarded the loaded library.")
            return
        assert (run / "export.jpg").read_bytes() == ASSET
        for expected in ("Prints library", "Fixture model", "Style", "Garden keeper", "Original exported"):
            assert expected in contents, expected
        assert ("listOutputs", {"recursive": True, "limit": 24, "folder": "Studies"}) in CALLS
        assert sum(operation == "getOutput" for operation, _ in CALLS) == 4
        assert any(operation == "openOutput" and request["byteOffset"] > 0 for operation, request in CALLS)
        print("Connected workspace replay passed. Exact original bytes exported from the fictional Oyster fixture.")


if __name__ == "__main__":
    main()
