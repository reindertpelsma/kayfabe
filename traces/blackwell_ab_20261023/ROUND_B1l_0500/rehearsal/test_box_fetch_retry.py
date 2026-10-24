"""B1l (controller 05:00): box_fetch retries a failed byte range and keeps completed ranges.

Run: KF_BW3_K=<run_kit> python3 -m unittest test_box_fetch_retry
"""

import hashlib
import http.server
import os
from pathlib import Path
import re
import sys
import tempfile
import threading
import time
import unittest

K = Path(os.environ.get("KF_BW3_K", "/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/"
                        "ROUND_B1l_0500/attempt1_m30498/run_kit"))
sys.path.insert(0, str(K))
import box_fetch as BF  # noqa: E402

BODY = os.urandom(8 * 40000)
WORKERS = 8
RANGES = BF.ranges(len(BODY), WORKERS)


class FlakyHandler(http.server.BaseHTTPRequestHandler):
    """Range k fails `fail_times` times (None = forever) in `mode` ('http500' or 'cut')."""

    k = 3
    fail_times = 1
    mode = "http500"
    counts = None
    lock = threading.Lock()

    def log_message(self, *args):
        pass

    def do_GET(self):
        a, b = map(int, re.fullmatch(r"bytes=(\d+)-(\d+)", self.headers["Range"]).groups())
        k_start, k_end = RANGES[self.k]
        in_k = k_start <= a <= k_end
        with self.lock:
            n = self.counts.get(self.k, 0)
            if in_k:
                self.counts[self.k] = n + 1
        failing = in_k and (self.fail_times is None or n < self.fail_times)
        part = BODY[a:b + 1]
        if failing and self.mode == "http500":
            self.send_response(500)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.send_response(206)
        self.send_header("Content-Range", "bytes %d-%d/%d" % (a, b, len(BODY)))
        self.send_header("Content-Length", str(len(part)))
        self.end_headers()
        if failing and self.mode == "cut":
            self.wfile.write(part[: len(part) // 2])
            self.wfile.flush()
            self.connection.shutdown(2)
            return
        self.wfile.write(part)


def serve(**kw):
    handler = type("H", (FlakyHandler,), dict(kw, counts={}))
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, handler, "http://127.0.0.1:%d/f.bin" % httpd.server_address[1]


def fetch(cutoff=40, **kw):
    httpd, handler, url = serve(**kw)
    try:
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            row = {"file": "f.bin", "url": url, "expected_bytes": len(BODY),
                   "expected_sha256": hashlib.sha256(BODY).hexdigest()}
            t0 = time.monotonic()
            st = BF.fetch_file(row, cache, time.monotonic() + cutoff, workers=WORKERS,
                               allow_http_for_test=True)
            took = time.monotonic() - t0
            part = cache / "f.bin.part"
            data = part.read_bytes() if part.exists() else None
            left = sorted(p.name for p in cache.iterdir())
        return st, data, left, handler.counts, took
    finally:
        httpd.shutdown()


class RangeRetry(unittest.TestCase):
    def setUp(self):
        self.saved = BF.BACKOFF_SECONDS
        BF.BACKOFF_SECONDS = (0.2, 0.4, 0.8)

    def tearDown(self):
        BF.BACKOFF_SECONDS = self.saved

    def test_policy_fits_the_cutoff(self):
        self.assertEqual(self.saved, (5, 15, 30))
        self.assertEqual(BF.RANGE_RETRIES, 3)
        self.assertLess(sum(self.saved), 600 / 4)

    def test_range_k_fails_once_then_verified(self):
        st, data, left, counts, _ = fetch(fail_times=1)
        self.assertEqual(st["state"], "verified", st)
        self.assertEqual(data, BODY)
        self.assertEqual(left, ["f.bin.part"])
        self.assertEqual(counts[3], 2)
        self.assertEqual(st["range_attempts"][3], 2)

    def test_range_k_fails_twice_then_verified(self):
        st, data, _, counts, _ = fetch(fail_times=2)
        self.assertEqual(st["state"], "verified", st)
        self.assertEqual(data, BODY)
        self.assertEqual(counts[3], 3)

    def test_range_k_fails_forever_publishes_nothing(self):
        st, data, left, counts, _ = fetch(fail_times=None)
        self.assertEqual(st["state"], "failed")
        self.assertIsNone(data)
        self.assertEqual(left, [])
        self.assertEqual(counts[3], 1 + BF.RANGE_RETRIES)

    def test_completed_ranges_are_not_downloaded_again(self):
        st, _, _, counts, _ = fetch(fail_times=2)
        self.assertEqual(st["state"], "verified")
        self.assertEqual(set(st["range_attempts"].values()) - {1}, {3})
        self.assertEqual(sum(1 for v in st["range_attempts"].values() if v == 1), WORKERS - 1)

    def test_cut_connection_resumes_from_kept_bytes(self):
        st, data, _, counts, _ = fetch(fail_times=1, mode="cut")
        self.assertEqual(st["state"], "verified", st)
        self.assertEqual(data, BODY)
        self.assertGreaterEqual(st["resumed_bytes"][3], 1)

    def test_cut_forever_fails_without_publishing(self):
        st, data, left, _, _ = fetch(fail_times=None, mode="cut")
        self.assertEqual(st["state"], "failed")
        self.assertIsNone(data)
        self.assertEqual(left, [])

    def test_retries_stop_at_the_cutoff(self):
        BF.BACKOFF_SECONDS = (5, 5, 5)
        st, _, _, _, took = fetch(fail_times=None, cutoff=2.5)
        self.assertEqual(st["state"], "failed")
        self.assertLess(took, 5)

    def test_sha_still_verified_on_the_assembled_file(self):
        httpd, handler, url = serve(fail_times=1)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                row = {"file": "f.bin", "url": url, "expected_bytes": len(BODY),
                       "expected_sha256": "0" * 64}
                st = BF.fetch_file(row, Path(tmp), time.monotonic() + 30, workers=WORKERS,
                                   allow_http_for_test=True)
                self.assertEqual(st["state"], "mismatch")
                self.assertEqual(sorted(p.name for p in Path(tmp).iterdir()), [])
        finally:
            httpd.shutdown()


class Fallback(unittest.TestCase):
    def test_whole_file_fallback_documented(self):
        text = (K / "box_fetch.py").read_text()
        self.assertIn("partial-range SSH fallback: not implemented", text)


if __name__ == "__main__":
    unittest.main()
