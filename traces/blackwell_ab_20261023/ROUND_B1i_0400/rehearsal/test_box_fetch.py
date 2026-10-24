"""B1i (controller 04:00): box-side media fetch from pinned public sources, SSH upload as fallback.

Run: KF_BW3_D=<workspace> KF_BW3_K=<run_kit> python3 -m unittest test_box_fetch
"""

import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import threading
import time
import unittest

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1i-a1_m30498-run"))
K = Path(
    os.environ.get(
        "KF_BW3_K",
        "/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/"
        "ROUND_B1i_0400/attempt1_m30498/run_kit",
    )
)
TRACE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(K))
sys.path.insert(0, str(D))
import box_fetch as BF  # noqa: E402
import media_transfer as MT  # noqa: E402

WIN_VM = subprocess.check_output(
    [
        "git",
        "-C",
        "/workspace/bw-native-edge-wt",
        "show",
        "9ff51a6e9:scripts/bench/windows/win_vm.sh",
    ],
    text=True,
)


def harness_table():
    rows = {}
    for m in re.finditer(r'^  "([^|"]+)\|([^|"]+)\|([0-9a-f]{64})\|(\d+)"$', WIN_VM, re.M):
        rows[m[1]] = {"url": m[2], "sha256": m[3], "bytes": int(m[4])}
    return rows


class Handler(http.server.BaseHTTPRequestHandler):
    body = b""
    honour_range = True
    delay = 0.0

    def log_message(self, *args):
        pass

    def do_GET(self):
        time.sleep(self.delay)
        data = self.body
        rng = self.headers.get("Range")
        if rng and self.honour_range:
            a, b = map(int, re.fullmatch(r"bytes=(\d+)-(\d+)", rng).groups())
            part = data[a : b + 1]
            self.send_response(206)
            self.send_header("Content-Range", "bytes %d-%d/%d" % (a, b, len(data)))
        else:
            part = data
            self.send_response(200)
        self.send_header("Content-Length", str(len(part)))
        self.end_headers()
        self.wfile.write(part)


class ShortRange(Handler):
    def do_GET(self):
        a, b = map(int, re.fullmatch(r"bytes=(\d+)-(\d+)", self.headers["Range"]).groups())
        part = self.body[a:b]
        self.send_response(206)
        self.send_header("Content-Range", "bytes %d-%d/%d" % (a, b - 1, len(self.body)))
        self.send_header("Content-Length", str(len(part)))
        self.end_headers()
        self.wfile.write(part)


class Server:
    def __init__(self, body, honour_range=True, delay=0.0):
        handler = type(
            "H", (Handler,), {"body": body, "honour_range": honour_range, "delay": delay}
        )
        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.url = "http://127.0.0.1:%d/file.bin" % self.httpd.server_address[1]
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()


def row(body, url, name="file.bin", sha=None):
    return {
        "file": name,
        "url": url,
        "expected_bytes": len(body),
        "expected_sha256": sha or hashlib.sha256(body).hexdigest(),
    }


class PinnedTable(unittest.TestCase):
    def test_manifest_equals_the_harness_win_vm_table(self):
        manifest = json.loads((D / "media_manifest.json").read_text())
        table = harness_table()
        self.assertEqual(len(table), 4)
        for f in manifest["files"]:
            pin = table[f["file"]]
            self.assertEqual(
                (f["url"], f["expected_sha256"], f["expected_bytes"]),
                (pin["url"], pin["sha256"], pin["bytes"]),
            )

    def test_windows_on_vast_pins_agree(self):
        wov = json.loads((TRACE / "windows_on_vast_pins.json").read_text())
        table = harness_table()
        nv = next(p for p in wov["pins"] if p["name"] == "nvidia.exe")
        exe = table["580.88-desktop-win10-win11-64bit-international-dch-whql.exe"]
        self.assertEqual((nv["url"], nv["sha256"]), (exe["url"], exe["sha256"]))
        msi = table["OpenSSH-Win64-v10.0.0.0.msi"]
        self.assertEqual(
            (wov["openssh"]["url"], wov["openssh"]["sha256"]), (msi["url"], msi["sha256"])
        )
        self.assertEqual(
            wov["iso_url_unpinned_there"], table["win11_ltsc_2024_eval_x64_en-us.iso"]["url"]
        )

    def test_box_fetch_uses_only_public_https_urls_from_the_manifest(self):
        manifest = json.loads((D / "media_manifest.json").read_text())
        for f in manifest["files"]:
            self.assertTrue(BF.allowed_url(f["url"]), f["url"])
        self.assertFalse(BF.allowed_url("http://example.com/x"))
        self.assertFalse(BF.allowed_url("file:///etc/passwd"))


class Fetch(unittest.TestCase):
    def fetch(
        self, body, served=None, honour_range=True, delay=0.0, cutoff=30, sha=None, workers=4
    ):
        server = Server(served if served is not None else body, honour_range, delay)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                cache = Path(tmp)
                r = row(body, server.url, sha=sha)
                status = BF.fetch_file(
                    r, cache, time.monotonic() + cutoff, workers=workers, allow_http_for_test=True
                )
                published = (cache / (r["file"] + ".part")).exists()
                leftovers = sorted(p.name for p in cache.iterdir())
                content = (cache / (r["file"] + ".part")).read_bytes() if published else None
            return status, published, leftovers, content
        finally:
            server.close()

    def test_ranged_download_verified_and_published_as_part(self):
        body = os.urandom(300000)
        status, published, leftovers, content = self.fetch(body)
        self.assertEqual(status["state"], "verified")
        self.assertTrue(published)
        self.assertEqual(content, body)
        self.assertEqual(leftovers, ["file.bin.part"])
        self.assertEqual(status["ranges"], 4)

    def test_sha_mismatch_refuses_and_publishes_nothing(self):
        body = os.urandom(200000)
        status, published, leftovers, _ = self.fetch(body, sha="0" * 64)
        self.assertEqual(status["state"], "mismatch")
        self.assertFalse(published)
        self.assertEqual(leftovers, [])

    def test_server_ignoring_range_fails_without_publishing(self):
        body = os.urandom(200000)
        status, published, leftovers, _ = self.fetch(body, honour_range=False)
        self.assertEqual(status["state"], "failed")
        self.assertFalse(published)
        self.assertEqual(leftovers, [])

    def test_wrong_size_body_fails(self):
        body = os.urandom(100000)
        status, published, _, _ = self.fetch(body, served=body[:-10], workers=1)
        self.assertIn(status["state"], ("failed", "mismatch"))
        self.assertFalse(published)

    def test_cutoff_stops_a_slow_source(self):
        body = os.urandom(50000)
        t0 = time.monotonic()
        status, published, leftovers, _ = self.fetch(body, delay=5, cutoff=1.5)
        self.assertLess(time.monotonic() - t0, 4.5)
        self.assertEqual(status["state"], "failed")
        self.assertFalse(published)
        self.assertEqual(leftovers, [])

    def test_status_file_written_for_all_files(self):
        body = os.urandom(120000)
        server = Server(body)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                tmp = Path(tmp)
                manifest = {
                    "all_verified": True,
                    "files": [row(body, server.url, "a.bin"), row(body, server.url, "b.bin")],
                }
                (tmp / "m.json").write_text(json.dumps(manifest))
                rc = BF.main(
                    [
                        str(tmp / "m.json"),
                        str(tmp / "cache"),
                        "20",
                        str(tmp / "st.json"),
                        "--allow-http-for-test",
                    ]
                )
                st = json.loads((tmp / "st.json").read_text())
            self.assertEqual(rc, 0)
            self.assertEqual(
                {k: v["state"] for k, v in st["files"].items()},
                {"a.bin": "verified", "b.bin": "verified"},
            )
            self.assertTrue(st["done"])
        finally:
            server.close()

    def test_corrupted_byte_same_size_is_a_mismatch(self):
        body = os.urandom(150000)
        bad = body[:70000] + bytes([body[70000] ^ 1]) + body[70001:]
        status, published, leftovers, _ = self.fetch(body, served=bad)
        self.assertEqual(status["state"], "mismatch")
        self.assertFalse(published)

    def test_short_range_answers_fail(self):
        body = os.urandom(150000)
        server = Server(body)
        server.httpd.RequestHandlerClass = type("S", (ShortRange,), {"body": body})
        try:
            with tempfile.TemporaryDirectory() as tmp:
                st = BF.fetch_file(
                    row(body, server.url),
                    Path(tmp),
                    time.monotonic() + 20,
                    workers=4,
                    allow_http_for_test=True,
                )
                self.assertEqual(st["state"], "failed")
                self.assertEqual(sorted(p.name for p in Path(tmp).iterdir()), [])
        finally:
            server.close()

    def test_single_stream_cutoff_is_enforced(self):
        body = os.urandom(10000)
        t0 = time.monotonic()
        status, published, _, _ = self.fetch(body, delay=6, cutoff=1.2, workers=1)
        self.assertLess(time.monotonic() - t0, 4.5)
        self.assertEqual(status["state"], "failed")

    def test_disallowed_url_fails_without_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = {
                "file": "x.iso",
                "url": "https://evil.example/x.iso",
                "expected_bytes": 10,
                "expected_sha256": "0" * 64,
            }
            st = BF.fetch_file(r, Path(tmp), time.monotonic() + 5)
            self.assertEqual(st["state"], "failed")
            self.assertEqual(st["ranges"], 0)


class Fallback(unittest.TestCase):
    def test_cutoff_arithmetic(self):
        self.assertEqual(MT.TOTAL_SECONDS, 1200)
        self.assertEqual(MT.BOX_CUTOFF_SECONDS, 600)
        self.assertLessEqual(MT.BOX_CUTOFF_SECONDS + MT.MIN_FALLBACK_SECONDS, MT.TOTAL_SECONDS)

    def test_verified_is_done(self):
        self.assertEqual(MT.decide({"state": "verified"}, 10, 100), "done")

    def test_failure_or_mismatch_falls_back_immediately(self):
        for state in ("failed", "mismatch"):
            self.assertEqual(MT.decide({"state": state}, 5, 100), "fallback")

    def test_cutoff_falls_back(self):
        st = {"state": "running", "bytes_done": 99, "bytes_total": 100}
        self.assertEqual(MT.decide(st, MT.BOX_CUTOFF_SECONDS, 100), "fallback")

    def test_complete_but_unverified_at_cutoff_falls_back(self):
        st = {"state": "running", "bytes_done": 1000, "bytes_total": 1000}
        self.assertEqual(MT.decide(st, MT.BOX_CUTOFF_SECONDS, 1000), "fallback")

    def test_slow_projection_falls_back_early(self):
        st = {"state": "running", "bytes_done": 10, "bytes_total": 1000}
        self.assertEqual(MT.decide(st, MT.PROJECTION_AFTER_SECONDS, 1000), "fallback")

    def test_fast_progress_waits(self):
        st = {"state": "running", "bytes_done": 800, "bytes_total": 1000}
        self.assertEqual(MT.decide(st, MT.PROJECTION_AFTER_SECONDS, 1000), "wait")

    def test_no_projection_before_threshold(self):
        st = {"state": "running", "bytes_done": 0, "bytes_total": 1000}
        self.assertEqual(MT.decide(st, MT.PROJECTION_AFTER_SECONDS - 1, 1000), "wait")

    def test_missing_status_waits_then_falls_back_at_cutoff(self):
        self.assertEqual(MT.decide(None, 30, 100), "wait")
        self.assertEqual(MT.decide(None, MT.BOX_CUTOFF_SECONDS, 100), "fallback")


class BoxPhase(unittest.TestCase):
    def run_phase(self, statuses, files=("a.iso", "b.exe")):
        clock = [0.0]
        calls = []
        seq = iter(statuses)

        def runner(cmd, timeout):
            calls.append(cmd)
            if "box_fetch.py" in cmd:
                return "4242\n"
            if cmd.startswith("touch "):
                return ""
            return json.dumps({"files": next(seq, statuses[-1]), "done": False})

        def sleep(s):
            clock[0] += s

        m = {"files": [{"file": f, "expected_bytes": 1000} for f in files]}
        result = MT.box_phase(runner, m, clock=lambda: clock[0], sleep=sleep, total=1200)
        return result, calls, clock[0]

    def test_all_verified_skips_upload(self):
        st = {"a.iso": {"state": "verified"}, "b.exe": {"state": "verified"}}
        r, calls, t = self.run_phase([st])
        self.assertEqual(r["verified"], {"a.iso", "b.exe"})
        self.assertEqual(r["fallback"], [])
        self.assertLess(t, 60)

    def test_mismatch_falls_back_and_stops_that_file(self):
        st = {"a.iso": {"state": "mismatch"}, "b.exe": {"state": "verified"}}
        r, calls, _ = self.run_phase([st])
        self.assertEqual(r["fallback"], ["a.iso"])
        self.assertTrue(any(c.startswith("touch ") and "a.iso.boxfetch.stop" in c for c in calls))

    def test_stalled_file_falls_back_by_the_cutoff(self):
        st = {
            "a.iso": {"state": "running", "bytes_done": 999, "bytes_total": 1000},
            "b.exe": {"state": "verified"},
        }
        r, _, t = self.run_phase([st])
        self.assertEqual(r["fallback"], ["a.iso"])
        self.assertLessEqual(t, MT.BOX_CUTOFF_SECONDS + MT.POLL_SECONDS)

    def test_unreadable_status_never_blocks_past_the_cutoff(self):
        clock = [0.0]

        def runner(cmd, timeout):
            if "box_fetch.py" in cmd:
                return "1\n"
            if cmd.startswith("touch "):
                return ""
            raise subprocess.TimeoutExpired(cmd, timeout)

        m = {"files": [{"file": "a.iso", "expected_bytes": 1000}]}
        r = MT.box_phase(
            runner,
            m,
            clock=lambda: clock[0],
            sleep=lambda s: clock.__setitem__(0, clock[0] + s),
            total=1200,
        )
        self.assertEqual(r["fallback"], ["a.iso"])
        self.assertLessEqual(clock[0], MT.BOX_CUTOFF_SECONDS + MT.POLL_SECONDS)

    def test_box_fetch_started_with_the_cutoff_and_public_manifest(self):
        st = {"a.iso": {"state": "verified"}, "b.exe": {"state": "verified"}}
        _, calls, _ = self.run_phase([st])
        start = next(c for c in calls if "box_fetch.py" in c)
        self.assertIn("/root/sweep2/media_manifest.json", start)
        self.assertIn(" %d " % MT.BOX_CUTOFF_SECONDS, start)

    def test_mismatch_falls_back_on_the_first_poll(self):
        st = {"a.iso": {"state": "mismatch"}, "b.exe": {"state": "verified"}}
        _, _, t = self.run_phase([st])
        self.assertLess(t, MT.POLL_SECONDS * 2)

    def test_slow_projection_falls_back_well_before_the_cutoff(self):
        st = {
            "a.iso": {"state": "running", "bytes_done": 10, "bytes_total": 1000},
            "b.exe": {"state": "verified"},
        }
        r, calls, t = self.run_phase([st])
        self.assertEqual(r["fallback"], ["a.iso"])
        self.assertLess(t, MT.PROJECTION_AFTER_SECONDS + 2 * MT.POLL_SECONDS)

    def test_stalled_fallback_also_stops_the_box_fetch(self):
        st = {
            "a.iso": {"state": "running", "bytes_done": 999, "bytes_total": 1000},
            "b.exe": {"state": "verified"},
        }
        _, calls, _ = self.run_phase([st])
        self.assertTrue(any(c.startswith("touch ") and "a.iso.boxfetch.stop" in c for c in calls))


class StopMarker(unittest.TestCase):
    def test_stop_marker_aborts_a_running_fetch(self):
        body = os.urandom(80000)
        server = Server(body, delay=2)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                cache = Path(tmp)
                (cache / "file.bin.boxfetch.stop").write_text("")
                st = BF.fetch_file(
                    row(body, server.url), cache, time.monotonic() + 20, allow_http_for_test=True
                )
                self.assertEqual(st["state"], "failed")
                self.assertFalse((cache / "file.bin.part").exists())
        finally:
            server.close()

    def test_stop_marker_written_mid_download_kills_curl_quickly(self):
        body = os.urandom(80000)
        server = Server(body, delay=8)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                cache = Path(tmp)
                threading.Timer(
                    0.7, lambda: (cache / "file.bin.boxfetch.stop").write_text("")
                ).start()
                t0 = time.monotonic()
                st = BF.fetch_file(
                    row(body, server.url),
                    cache,
                    time.monotonic() + 30,
                    workers=2,
                    allow_http_for_test=True,
                )
                self.assertLess(time.monotonic() - t0, 4)
                self.assertEqual(st["state"], "failed")
        finally:
            server.close()

    def test_stop_marker_mid_download_single_stream(self):
        body = os.urandom(20000)
        server = Server(body, delay=8)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                cache = Path(tmp)
                threading.Timer(
                    0.7, lambda: (cache / "file.bin.boxfetch.stop").write_text("")
                ).start()
                t0 = time.monotonic()
                st = BF.fetch_file(
                    row(body, server.url),
                    cache,
                    time.monotonic() + 30,
                    workers=1,
                    allow_http_for_test=True,
                )
                self.assertLess(time.monotonic() - t0, 4)
                self.assertEqual(st["state"], "failed")
        finally:
            server.close()


class Wiring(unittest.TestCase):
    def test_parallel_eight_uploads_only_fallback_files_within_the_same_deadline(self):
        src = (D / "parallel_eight.py").read_text()
        self.assertIn("box = MT.box_phase(", src)
        self.assertIn("if row['file'] in box['verified']:", src)
        self.assertIn("seconds=1200", src)
        self.assertLess(src.index("box = MT.box_phase("), src.index("for row in m['files']:"))

    def test_verified_files_are_never_uploaded(self):
        src = (D / "parallel_eight.py").read_text()
        block = src[src.index("    for row in m['files']:\n") :]
        self.assertTrue(
            block.startswith(
                "    for row in m['files']:\n"
                "        if row['file'] in box['verified']:\n"
                "            continue\n"
                "        start_upload(row['file'])\n"
            )
        )

    def test_box_fetch_is_in_the_bundle_and_secret_free(self):
        text = (K / "box_fetch.py").read_text()
        for bad in ("vast_api_key", "BEGIN OPENSSH", "Authorization", "token"):
            self.assertNotIn(bad, text)
        self.assertTrue((K / "box_fetch.py").exists())

    def test_dry_loop_covers_box_fetch(self):
        sys.path.insert(0, "/workspace/bw3-round-b1g-common")
        import dry_loop as DL

        self.assertIn("test_box_fetch", {s for _, s in DL.SUITES})


if __name__ == "__main__":
    unittest.main()
