#!/usr/bin/env python3
"""Exercise the native Prints library against a loopback-only fictional Oyster.

The host, key, outputs and assets here are fictional and app-owned. Mode flags
hold or fail optional model discovery without changing the required library
read, so the replay can prove that library work is independent of it.
"""
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
# A second, distinct output proves an export stays bound to the item chosen
# when the transfer started even if the visible selection moves on.
ALT_ASSET = ASSET + b"\nsecond fictional output"
KEY = "oyk_fictional_local_app_key"
CALLS = []
EVENTS = []
STARTED = time.monotonic()
# Injected only into failure payloads and private record fields. None of it may
# reach a reviewable connection report.
SENSITIVE = [
    "oyk_injected_secret_9182",
    "/private/owners/secret.jpg",
    "computer-private-42",
    "A private prompt never to print",
    "Authorization: Bearer",
]


def output(index=0, computer=None, prompt=None, path=None):
    asset = ALT_ASSET if index else ASSET
    default_path = "/library/Studies/second.jpg" if index else "/library/Studies/horizon.jpg"
    return {
        "path": path or ("/private/owners/secret.jpg" if OysterFixture.inject else default_path),
        "print": {
            "printId": ("print_fixture_2" if index else "print_fixture"),
            "computerId": computer or ("computer-private-42" if OysterFixture.inject else "fictional-computer"),
            "kind": "image",
            "model": "fictional-image-model",
            "format": "jpg",
            "bytes": 0 if OysterFixture.unknown_total else len(asset),
            "width": 1280,
            "height": 720,
            "createdAt": 1730000000000 + index,
        },
        "characters": ["Garden keeper"],
        "style": "Field notes",
        "typed": {
            "prompt": prompt or ("A private prompt never to print" if OysterFixture.inject else "A fictional landscape from a local fixture."),
            "references": 1,
        },
    }


class OysterFixture(BaseHTTPRequestHandler):
    revoke = False
    legacy = False
    transient = False
    folder_error = False
    keep_local = False
    library_unavailable = False
    catalogue_delay = 0.0
    catalogue_fail = None
    catalogue_retry = False
    late = False
    inject = False
    attempts = 0
    first_model_request = None
    run_started = None
    catalogue_hold = False
    catalogue_release = None
    late_second = None
    # Original-transfer controls. Holding a chunk leaves a request in flight
    # while the native replay can ask the worker to stop before the next one.
    chunk_delay = 0.0
    chunk_hold_from = None
    chunk_release = None
    chunk_fail_at = None
    chunk_failed = False
    chunk_revoke_at = None
    chunk_count = 0
    unknown_total = False

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
        if operation == "listModels":
            # Count on the handler class: each request gets a fresh instance.
            type(self).attempts += 1
            attempt = type(self).attempts
            if self.catalogue_fail and (attempt == 1 or not self.catalogue_retry):
                self.catalogue_reply(failed=True)
                return
            delay = self.catalogue_delay
            release_at = None
            if type(self).catalogue_hold and attempt == 1:
                # Deterministic pending state: the answer is held until the
                # native replay has exited, then released for a clean shutdown.
                if type(self).catalogue_release is not None:
                    type(self).catalogue_release.wait(timeout=20)
            elif self.late and attempt == 1:
                # Hold the superseded answer until the reconnected client has
                # actually started its own catalogue request.
                if type(self).late_second is not None:
                    type(self).late_second.wait(timeout=20)
                if type(self).first_model_request is None:
                    type(self).first_model_request = time.monotonic()
            elif self.late and attempt > 1:
                if type(self).late_second is not None:
                    type(self).late_second.set()
            elif delay:
                release_at = time.monotonic() + delay
            if release_at is not None:
                time.sleep(max(0.0, release_at - time.monotonic()))
            self.catalogue_reply(failed=False, stale=self.late and attempt == 1)
            return
        if operation == "listOutputs":
            if self.library_unavailable:
                self.reply(200, {"error": "library_unavailable", "message": "Untrusted fixture text"})
            elif "recursive" in arguments and (
                self.legacy or (self.transient and len(CALLS) == 1)
            ):
                self.reply(502, {"error": "App execution failed"})
            elif self.revoke and arguments.get("folder") == "Studies":
                self.reply(200, {"error": "revoked"})
            elif self.folder_error and arguments.get("folder") == "Studies":
                self.reply(502, {"error": "App execution failed"})
            else:
                self.reply(200, {"items": [output(0), output(1)], "next": None})
        elif operation == "listFolders":
            if self.legacy:
                self.reply(403, {"error": "not_declared"})
            else:
                self.reply(200, {"folders": [{"name": "Studies", "folder": "Studies"}], "next": None})
        elif operation == "getOutput":
            index = 1 if "second.jpg" in arguments["path"] else 0
            self.reply(200, output(index))
        elif operation == "openOutput":
            index = 1 if arguments["printId"] == "print_fixture_2" else 0
            asset = ALT_ASSET if index else ASSET
            offset = arguments["byteOffset"]
            type(self).chunk_count += 1
            if (
                type(self).chunk_hold_from is not None
                and type(self).chunk_count >= type(self).chunk_hold_from
                and type(self).chunk_release is not None
            ):
                type(self).chunk_release.wait(timeout=0.8)
            if (
                type(self).chunk_fail_at is not None
                and offset == type(self).chunk_fail_at
                and not type(self).chunk_failed
            ):
                type(self).chunk_failed = True
                self.reply(502, {"error": "App execution failed", "message": " | ".join(SENSITIVE)})
                origin = type(self).run_started or STARTED
                EVENTS.append(("chunk_failed", round(time.monotonic() - origin, 3), str(offset)))
                return
            if type(self).chunk_revoke_at is not None and offset == type(self).chunk_revoke_at:
                self.reply(200, {"error": "revoked", "message": " | ".join(SENSITIVE)})
                return
            if type(self).chunk_delay:
                time.sleep(type(self).chunk_delay)
            chunk = asset[offset : offset + arguments["byteLength"]]
            self.reply(200, {
                "path": output(index)["path"],
                "printId": arguments["printId"],
                "byteOffset": offset,
                "content": base64.b64encode(chunk).decode(),
                "encoding": "base64",
                "truncated": bool(chunk),
            })
            origin = type(self).run_started or STARTED
            EVENTS.append(("chunk_replied", round(time.monotonic() - origin, 3), str(offset)))
        else:
            self.reply(404, {"error": "not_found"})

    def catalogue_reply(self, failed, stale=False):
        if failed:
            mode = self.catalogue_fail
            message = " | ".join(SENSITIVE)
            if mode == "temporary":
                status, body = 502, {"error": "App execution failed", "message": message}
            elif mode == "malformed":
                status, body = 200, {"models": "not-a-list", "message": message}
            elif mode == "permission":
                status, body = 200, {"error": "not_granted", "message": message}
            else:
                status, body = 200, {"error": "revoked", "message": message}
        else:
            status, body = 200, {
                "models": [
                    {
                        "id": "fictional-image-model",
                        "label": "Obsolete fixture model" if stale else "Fixture model",
                        "kind": "image",
                    }
                ]
            }
        self.reply(status, body)
        origin = type(self).run_started or STARTED
        EVENTS.append((
            "catalogue_completed",
            round(time.monotonic() - origin, 3),
            "failed" if failed else "ok",
        ))

    def reply(self, status, body):
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)
        if self.path.endswith("/listOutputs") and status == 200 and isinstance(body, dict) and "items" in body:
            origin = type(self).run_started or STARTED
            EVENTS.append(("library_read_completed", round(time.monotonic() - origin, 3), "items"))


def has_key(node, key):
    if isinstance(node, dict):
        return node.get("key") == key or any(has_key(value, key) for value in node.values())
    if isinstance(node, list):
        return any(has_key(value, key) for value in node)
    return False


def report_text(nodes):
    # Keep the check inside the report card so prompts and private record
    # fields elsewhere in the accessibility tree cannot mask an exclusion.
    if isinstance(nodes, dict):
        if nodes.get("key") == "connection-report-text":
            report = nodes.get("text") or nodes.get("name")
            if report:
                return report
        children = nodes.values()
    elif isinstance(nodes, list):
        children = nodes
    else:
        return None
    return next((report for node in children if (report := report_text(node))), None)


def assert_report_safe(report):
    assert report is not None, "the reviewable connection report was not visible"
    assert "Prints Desktop connection report" in report
    assert "App:" in report and "Platform:" in report and "Trail:" in report
    assert "op-" in report, "opaque correlation IDs are missing"
    assert "ms" in report, "elapsed duration is missing"
    for secret in SENSITIVE + [KEY]:
        assert secret not in report, f"connection report leaked {secret}"


def first(name, events=None):
    entries = EVENTS if events is None else events
    return next((value for entry in entries if entry[0] == name for value in [entry[1]]), None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", nargs="?", type=Path)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--revoked", action="store_true")
    modes.add_argument("--legacy", action="store_true")
    modes.add_argument("--transient", action="store_true")
    modes.add_argument("--folder-error", action="store_true")
    modes.add_argument("--keep-local", action="store_true")
    modes.add_argument("--library-unavailable", action="store_true")
    modes.add_argument("--catalogue-pending-live", action="store_true")
    modes.add_argument("--catalogue-pending-export", action="store_true")
    modes.add_argument("--catalogue-fail", choices=["temporary", "malformed", "permission", "revoked"])
    modes.add_argument("--catalogue-retry", action="store_true")
    modes.add_argument("--late-catalogue", action="store_true")
    modes.add_argument("--transfer-progress", action="store_true")
    modes.add_argument("--transfer-unknown-total", action="store_true")
    modes.add_argument("--transfer-browse", action="store_true")
    modes.add_argument("--transfer-cancel", action="store_true")
    modes.add_argument("--transfer-retry", action="store_true")
    modes.add_argument("--transfer-revoked", action="store_true")
    options = parser.parse_args()
    OysterFixture.revoke = options.revoked
    OysterFixture.legacy = options.legacy
    OysterFixture.transient = options.transient
    OysterFixture.folder_error = options.folder_error
    OysterFixture.keep_local = options.keep_local
    OysterFixture.library_unavailable = options.library_unavailable
    OysterFixture.catalogue_fail = options.catalogue_fail or ("temporary" if options.catalogue_retry else None)
    OysterFixture.catalogue_retry = options.catalogue_retry
    OysterFixture.late = options.late_catalogue
    OysterFixture.inject = bool(
        options.catalogue_fail or options.catalogue_retry or options.late_catalogue
    )
    OysterFixture.catalogue_delay = 0.0
    OysterFixture.catalogue_hold = options.catalogue_pending_live or options.catalogue_pending_export
    OysterFixture.catalogue_release = threading.Event()
    OysterFixture.late_second = threading.Event() if options.late_catalogue else None
    OysterFixture.chunk_delay = (
        0.35 if options.transfer_progress or options.transfer_unknown_total
        else 0.30 if options.transfer_browse
        else 0.18 if options.transfer_retry
        else 0.0
    )
    OysterFixture.chunk_hold_from = 2 if options.transfer_cancel else None
    OysterFixture.chunk_release = threading.Event() if options.transfer_cancel else None
    OysterFixture.chunk_fail_at = 131072 if options.transfer_retry else None
    OysterFixture.chunk_failed = False
    OysterFixture.chunk_revoke_at = 131072 if options.transfer_revoked else None
    OysterFixture.chunk_count = 0
    OysterFixture.unknown_total = options.transfer_unknown_total
    OysterFixture.attempts = 0
    OysterFixture.first_model_request = None
    OysterFixture.run_started = None
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
            ]
            finish = "6.45"
            export_path = run / "export.jpg"
            if options.catalogue_fail == "revoked":
                actions += [
                    (3.30, "tap", {"key": "report-details"}),
                    (3.80, "tap", {"key": "copy-report"}),
                ]
                finish = "4.30"
            elif options.catalogue_fail:
                actions += [
                    (3.30, "tap", {"key": "report-details"}),
                    (3.80, "tap", {"key": "copy-report"}),
                    (4.20, "tap", {"key": "hosted-preview"}),
                    (4.80, "tap", {"key": "hosted-save"}),
                    (5.40, "file-dialog", {"path": str(export_path)}),
                ]
                finish = "5.90"
            elif options.catalogue_retry:
                actions += [
                    (3.20, "tap", {"key": "browse-folders"}),
                    (3.70, "tap", {"key": "folder-0"}),
                    (4.20, "tap", {"key": "hosted-item-0"}),
                    (4.70, "tap", {"key": "catalogue-retry"}),
                    (5.05, "tap", {"key": "report-details"}),
                    (5.35, "tap", {"key": "hosted-preview"}),
                    (5.90, "tap", {"key": "hosted-save"}),
                    (6.60, "file-dialog", {"path": str(export_path)}),
                ]
                finish = "7.20"
            elif options.late_catalogue:
                actions += [
                    # Disconnect lives on the connection screen, not the
                    # library. Disconnect keeps the origin and installation.
                    (3.20, "tap", {"key": "connection"}),
                    (3.60, "tap", {"key": "disconnect-hosted"}),
                    (4.40, "tap", {"key": "prints-key"}),
                    (4.55, "type", {"text": KEY}),
                    (5.75, "tap", {"key": "connect-hosted"}),
                    (6.60, "tap", {"key": "report-details"}),
                ]
                finish = "7.10"
            elif options.catalogue_pending_live:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.20, "tap", {"key": "report-details"}),
                ]
                finish = "4.50"
            elif options.catalogue_pending_export:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.40, "tap", {"key": "hosted-preview"}),
                    (5.15, "tap", {"key": "hosted-save"}),
                    (5.60, "tap", {"key": "browse-folders"}),
                    (5.90, "file-dialog", {"path": str(export_path)}),
                    (6.20, "tap", {"key": "report-details"}),
                ]
                finish = "6.55"
            elif options.transfer_progress or options.transfer_unknown_total:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.40, "tap", {"key": "hosted-save"}),
                    (4.85, "tap", {"key": "hosted-item-1"}),
                    (5.10, "tap", {"key": "report-details"}),
                ]
                finish = "5.35"
            elif options.transfer_browse:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.40, "tap", {"key": "hosted-save"}),
                    (4.85, "tap", {"key": "browse-folders"}),
                    (5.15, "tap", {"key": "folder-0"}),
                    (5.45, "tap", {"key": "hosted-item-1"}),
                    (6.40, "file-dialog", {"path": str(export_path)}),
                    (6.70, "tap", {"key": "report-details"}),
                ]
                finish = "7.00"
            elif options.transfer_cancel:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.40, "tap", {"key": "hosted-save"}),
                    (4.85, "tap", {"key": "hosted-transfer-cancel"}),
                    (5.60, "tap", {"key": "report-details"}),
                ]
                finish = "6.20"
            elif options.transfer_retry:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.40, "tap", {"key": "hosted-save"}),
                    (5.10, "tap", {"key": "hosted-item-1"}),
                    (5.45, "tap", {"key": "hosted-transfer-retry"}),
                    (6.90, "file-dialog", {"path": str(export_path)}),
                    (7.15, "tap", {"key": "report-details"}),
                ]
                finish = "7.45"
            elif options.transfer_revoked:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.40, "tap", {"key": "hosted-save"}),
                    (5.10, "tap", {"key": "report-details"}),
                ]
                finish = "5.50"
            else:
                actions += [
                    (3.40, "tap", {"key": "browse-folders"}),
                    (3.90, "tap", {"key": "folder-0"}),
                    (4.40, "tap", {"key": "hosted-preview"}),
                    (5.15, "tap", {"key": "hosted-save"}),
                    (5.60, "tap", {"key": "browse-folders"}),
                    (5.90, "file-dialog", {"path": str(export_path)}),
                ]
                if options.revoked or options.folder_error:
                    actions = [entry for entry in actions if entry[0] <= 3.90]
                elif options.library_unavailable:
                    actions = [entry for entry in actions if entry[0] <= 2.75]
                elif options.keep_local:
                    actions = [entry for entry in actions if entry[0] <= 2.75]
                    actions.append((3.0, "tap", {"key": "afterlight"}))
                elif options.legacy:
                    actions = [entry for entry in actions if entry[0] not in (3.40, 3.90, 5.60)]
                finish = "4.6" if (
                    options.revoked or options.folder_error or options.keep_local or options.library_unavailable
                ) else "6.45"
            steps = run / "steps.json"
            steps.write_text(json.dumps([{"t": t, "type": kind, **data} for t, kind, data in actions]))
            tree = run / "tree.json"
            args = [str(BINARY), "--script", str(steps), "--tree", str(tree), "--at", finish,
                    "--size", "1100x740", "--settle", "--realtime"]
            if options.snapshot:
                args.extend(["--snap", str(options.snapshot.resolve()), "--scale", "1"])
            # Time HTTP completions from the native run, not from import.
            OysterFixture.run_started = time.monotonic()
            subprocess.run(args, cwd=ROOT, check=True, timeout=45)
            # Snapshot at capture: the held catalogue is released afterwards
            # only so the loopback server can shut down cleanly.
            capture_events = list(EVENTS)
            if OysterFixture.catalogue_release is not None:
                OysterFixture.catalogue_release.set()
            if OysterFixture.late_second is not None:
                OysterFixture.late_second.set()
            if OysterFixture.chunk_release is not None:
                OysterFixture.chunk_release.set()
            server.shutdown()

        contents = tree.read_text()
        nodes = json.loads(contents)
        assert KEY not in contents
        if options.library_unavailable:
            assert "Choose a gallery folder in web Prints" in contents
            assert "selected Oyster Computer" in contents
            assert "Untrusted fixture text" not in contents
            assert has_key(nodes, "connect-hosted")
            assert has_key(nodes, "setup-guide")
            assert CALLS == [("listOutputs", {"recursive": True, "limit": 24})]
            print("First-run replay passed. Missing gallery shows recovery steps and allows reconnect.")
            return
        if options.revoked:
            assert "The key was refused or revoked" in contents
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
        if options.catalogue_pending_live:
            assert has_key(nodes, "hosted-item-0"), "library was not usable during discovery"
            assert has_key(nodes, "hosted-next") is False
            assert "Discovering models" in contents
            assert first("library_read_completed") is not None
            assert first("catalogue_completed", capture_events) is None, "catalogue finished before the capture"
            assert sum(operation == "listModels" for operation, _ in CALLS) == 1
            print(
                "Pending-catalogue replay passed. HTTP listOutputs completed at "
                f"{first('library_read_completed')}s; native library controls were present at the "
                f"{finish}s capture; catalogue had not completed."
            )
            return
        if options.catalogue_pending_export:
            assert (export_path.read_bytes()) == ASSET
            assert has_key(nodes, "hosted-item-0")
            assert "Discovering models" in contents
            assert "Original exported" in contents
            assert sum(operation == "getOutput" for operation, _ in CALLS) == 4
            assert first("catalogue_completed", capture_events) is None, "catalogue finished before the export"
            print(
                f"Pending-catalogue export replay passed. HTTP listOutputs completed at "
                f"{first('library_read_completed')}s; native export control was present at the "
                f"{finish}s capture; catalogue still pending while the exact original was written."
            )
            return
        if options.catalogue_fail:
            report = report_text(nodes)
            assert_report_safe(report)
            assert has_key(nodes, "copy-report")
            if options.catalogue_fail == "revoked":
                assert not has_key(nodes, "hosted-item-0")
                assert "Reconnect with a current scoped App key" in contents
                assert "still available" not in report
                assert all(operation != "openOutput" for operation, _ in CALLS)
            else:
                assert "Model catalogue unavailable for this connection." in contents
                assert has_key(nodes, "catalogue-retry")
                assert export_path.read_bytes() == ASSET
                if options.catalogue_fail == "temporary":
                    assert "http 502" in report
                    assert "temporarily unavailable" in contents
                elif options.catalogue_fail == "malformed":
                    assert "malformed" in contents
                elif options.catalogue_fail == "permission":
                    assert "media.generation" in contents
            print(
                f"Catalogue {options.catalogue_fail} replay passed. Friendly recovery, retry control, "
                "exact export and a safe copyable report were verified."
            )
            return
        if options.catalogue_retry:
            assert "Fixture model" in contents
            assert export_path.read_bytes() == ASSET
            assert has_key(nodes, "hosted-item-0")
            assert ("listOutputs", {"recursive": True, "limit": 24, "folder": "Studies"}) in CALLS
            assert sum(operation == "listOutputs" for operation, _ in CALLS) == 2
            assert sum(operation == "listModels" for operation, _ in CALLS) == 2
            # No second library read after retry: the page and its selected
            # output are untouched. Preview and export still use that item.
            assert sum(operation == "getOutput" for operation, _ in CALLS) == 4
            report = report_text(nodes)
            assert_report_safe(report)
            assert "http 502" in report
            print(
                f"Catalogue retry replay passed. Page and selection survived; library ready at "
                f"{first('library_read_completed')}s, catalogue completed at {first('catalogue_completed')}s."
            )
            return
        if options.late_catalogue:
            assert "Fixture model" in contents
            assert sum(operation == "listModels" for operation, _ in CALLS) == 2
            report = report_text(nodes)
            assert_report_safe(report)
            assert "session invalidated" in report
            assert "transport not known to be cancelled" in report
            assert "Obsolete fixture model" not in contents
            print(
                "Late-catalogue replay passed. The old answer was invalidated by disconnect/reconnect; "
                f"new library at {first('library_read_completed')}s, new catalogue at "
                f"{first('catalogue_completed')}s."
            )
            return
        if options.transfer_progress or options.transfer_unknown_total:
            report = report_text(nodes)
            assert_report_safe(report)
            assert has_key(nodes, "hosted-transfer-cancel"), "a stop action must be visible"
            assert has_key(nodes, "hosted-item-1"), "browsing must stay usable during a transfer"
            assert "Fetching original" in contents
            assert "bytes received" in contents
            assert "100%" not in contents, "an unfinished transfer must not claim completion"
            assert first("chunk_replied", capture_events) is not None, "no chunk was observed"
            if options.transfer_unknown_total:
                assert "total unknown" in contents
                assert f"of {len(ASSET)} bytes" not in contents
            else:
                assert f"of {len(ASSET)} bytes received" in contents
            print(
                f"Transfer-progress replay passed. First decoded chunk at "
                f"{first('chunk_replied')}s; observed bytes shown at the {finish}s capture "
                "while the library stayed browsable."
            )
            return
        if options.transfer_browse:
            assert (run / "export.jpg").read_bytes() == ASSET, "export must match the chosen original"
            assert has_key(nodes, "hosted-item-1"), "selection moved during the transfer"
            assert "Original exported" in contents
            ids = {request["printId"] for operation, request in CALLS if operation == "openOutput"}
            assert ids == {"print_fixture"}, f"transfer identity changed: {ids}"
            assert not any(
                operation == "getOutput" and request["path"].endswith("second.jpg")
                for operation, request in CALLS
            )
            report = report_text(nodes)
            assert_report_safe(report)
            print(
                "Transfer-browse replay passed. Browsing and selection changed mid-transfer; "
                f"first chunk at {first('chunk_replied')}s and the exact chosen original was written."
            )
            return
        if options.transfer_cancel:
            assert not (run / "export.jpg").exists(), "a cancelled transfer must not write a file"
            assert "Stopping the transfer" in contents or "Transfer stopped" in contents
            report = report_text(nodes)
            assert_report_safe(report)
            assert "in-flight request not known to be cancelled" in report
            assert OysterFixture.chunk_count == 2, OysterFixture.chunk_count
            assert any(event == "chunk_replied" and offset == "131072" for event, _, offset in capture_events)
            assert has_key(nodes, "hosted-item-1"), "the readable page survived cancellation"
            print(
                "Transfer-cancel replay passed. The in-flight reply returned before capture, "
                f"and the app stopped at {OysterFixture.chunk_count} chunk requests; "
                "no Save dialog and no file were produced."
            )
            return
        if options.transfer_retry:
            assert (run / "export.jpg").read_bytes() == ASSET
            assert "Original exported" in contents
            assert has_key(nodes, "hosted-item-0"), "the readable page survived the failure"
            report = report_text(nodes)
            assert_report_safe(report)
            assert "http 502" in report
            assert OysterFixture.chunk_count > 5, OysterFixture.chunk_count
            print(
                f"Transfer-retry replay passed. Failure at {first('chunk_failed')}s kept the page; "
                f"retry used {OysterFixture.chunk_count} chunk requests and wrote the exact original."
            )
            return
        if options.transfer_revoked:
            assert not (run / "export.jpg").exists(), "a revoked transfer must not write a file"
            assert has_key(nodes, "hosted-item-0") is False
            assert "key was refused or revoked" in contents
            report = report_text(nodes)
            assert_report_safe(report)
            assert OysterFixture.chunk_count <= 2, OysterFixture.chunk_count
            print(
                "Transfer-revocation replay passed. Authority loss cleared the library and "
                "no stale Save continuation ran."
            )
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
