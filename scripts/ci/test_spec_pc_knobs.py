#!/usr/bin/env python3
"""PC-run helpers of the SPECviewperf stage (scripts/bench/specviewperf/lanes_pc.json, stream_url.py), GPU-free.

FALSIFIER: lanes_pc.json differs from lanes.json outside the added pc lanes (the Vast payload reads lanes.json and
requires exactly lanea and laneb, so the pc lanes must never go there); a pc lane names an unknown viewset; the
streamer exits 0 for a body whose sha256 differs from the pin, or 3 for one that matches.
"""
import hashlib
import http.server
import json
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

SPEC = Path(__file__).resolve().parent.parent / "bench/specviewperf"
BODY = bytes(range(256)) * 5000


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)


class PcKnobs(unittest.TestCase):
    def test_table(self):
        base = json.loads((SPEC / "lanes.json").read_text())
        pc = json.loads((SPEC / "lanes_pc.json").read_text())
        self.assertEqual(sorted(base["lanes"]), ["lanea", "laneb"])
        extra = {k for k in pc["lanes"] if k not in base["lanes"]}
        self.assertEqual(extra, {"pcp1", "pcp2", "pcp3"})
        for k in base["lanes"]:
            self.assertEqual(pc["lanes"][k], base["lanes"][k])
        for k in extra:
            self.assertTrue(set(pc["lanes"][k]) <= set(pc["viewsets"]))
            self.assertIn(k, pc["min_free_gb"])
        for k in pc:
            if k not in ("lanes", "min_free_gb"):
                self.assertEqual(pc[k], base[k], k)

    def run_stream(self, pin, shape="flat", key="k", ref="k", extra=None, pin_args=None):
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory() as d:
                uf = Path(d) / "u.json"
                url = "http://127.0.0.1:%d/o" % srv.server_port
                doc = {"k": url} if shape == "flat" else {"schema": "kayfabe.download-urls/1", "urls": {ref: url}}
                if shape == "wrong-schema":
                    doc = {"schema": "kayfabe.download-urls/2", "urls": {ref: url}}
                if shape == "flat-with-schema-entry":
                    doc = {"schema": "not-a-url-but-a-flat-map-entry", "k": url}
                if extra:
                    (doc["urls"] if "urls" in doc else doc).update(extra)
                uf.write_text(json.dumps(doc))
                st = Path(d) / "st"
                p = subprocess.run([sys.executable, "-I", str(SPEC / "stream_url.py"), "--url-file", str(uf), "--key", key,
                                    *(["--sha256", pin] if pin_args is None else pin_args), "--status", str(st)], capture_output=True, timeout=60)
                return p.returncode, p.stdout, st.read_text() if st.exists() else ""
        finally:
            srv.shutdown()

    # 2026-11-16: the standalone release run's file (scripts/ci/box_downloader.py --urls, schema kayfabe.download-urls/1)
    def test_stream_reads_the_standalone_url_file_by_basename_of_the_key(self):
        rc, out, st = self.run_stream(hashlib.sha256(BODY).hexdigest(), "standalone", key="specviewperf-15.0.0/x.tar.zst", ref="x.tar.zst")
        self.assertEqual((rc, out), (0, BODY))
        self.assertTrue(st.startswith("STREAM_OK"))

    def test_stream_reads_the_standalone_url_file_by_the_full_key_too(self):
        rc, out, _ = self.run_stream(hashlib.sha256(BODY).hexdigest(), "standalone", key="a/b", ref="a/b")
        self.assertEqual((rc, out), (0, BODY))

    def test_stream_still_checks_the_pin_with_the_standalone_file(self):
        rc, _, st = self.run_stream("0" * 64, "standalone", key="d/k", ref="k")
        self.assertEqual(rc, 3)
        self.assertTrue(st.startswith("STREAM_MISMATCH"))

    def test_stream_refuses_a_standalone_file_without_the_object(self):
        rc, out, _ = self.run_stream(hashlib.sha256(BODY).hexdigest(), "standalone", key="d/missing", ref="k")
        self.assertNotEqual(rc, 0)
        self.assertEqual(out, b"")

    def test_the_full_key_wins_over_the_basename_when_both_are_present(self):
        rc, out, _ = self.run_stream(hashlib.sha256(BODY).hexdigest(), "standalone", key="a/b", ref="a/b", extra={"b": "http://127.0.0.1:9/dead"})
        self.assertEqual((rc, out), (0, BODY))

    def test_stream_refuses_a_standalone_file_with_only_other_objects(self):
        rc, out, _ = self.run_stream(hashlib.sha256(BODY).hexdigest(), "standalone", key="d/missing", ref="other1", extra={"other2": "http://127.0.0.1:9/x"})
        self.assertNotEqual(rc, 0)
        self.assertEqual(out, b"")

    def test_a_flat_map_may_have_an_entry_called_schema(self):
        rc, out, _ = self.run_stream(hashlib.sha256(BODY).hexdigest(), "flat-with-schema-entry", key="k")
        self.assertEqual((rc, out), (0, BODY))

    def test_a_document_with_another_schema_is_not_read_as_a_map(self):
        rc, out, _ = self.run_stream(hashlib.sha256(BODY).hexdigest(), "wrong-schema", key="k", ref="k")
        self.assertNotEqual(rc, 0)
        self.assertEqual(out, b"")

    def test_stream_match(self):
        rc, out, st = self.run_stream(hashlib.sha256(BODY).hexdigest())
        self.assertEqual((rc, out), (0, BODY))
        self.assertTrue(st.startswith("STREAM_OK"))

    # 2026-11-17 (OWNER_RULINGS section AX): the user's URL file is trusted; our git pin is only compared unless it is enforced
    def test_a_noted_pin_that_differs_is_recorded_and_never_a_refusal(self):
        rc, out, st = self.run_stream(None, pin_args=["--note-pin", "0" * 64])
        self.assertEqual((rc, out), (0, BODY))
        self.assertTrue(st.startswith("STREAM_OK"), st)
        self.assertIn("pin=differs", st)
        self.assertIn("sha256=" + hashlib.sha256(BODY).hexdigest(), st)

    def test_a_noted_pin_that_differs_still_delivers_every_byte_and_the_hash_of_what_was_delivered(self):
        rc, out, st = self.run_stream(None, pin_args=["--note-pin", "f" * 64])
        self.assertEqual(rc, 0)
        self.assertEqual(hashlib.sha256(out).hexdigest(), hashlib.sha256(BODY).hexdigest())
        self.assertIn("bytes=%d " % len(BODY), st)

    def test_a_noted_pin_that_matches_is_recorded_as_a_match(self):
        rc, out, st = self.run_stream(None, pin_args=["--note-pin", hashlib.sha256(BODY).hexdigest()])
        self.assertEqual((rc, out), (0, BODY))
        self.assertIn("pin=match", st)

    def test_no_pin_at_all_records_the_computed_hash_and_exits_zero(self):
        rc, out, st = self.run_stream(None, pin_args=[])
        self.assertEqual((rc, out), (0, BODY))
        self.assertIn("pin=none", st)
        self.assertIn("sha256=" + hashlib.sha256(BODY).hexdigest(), st)

    def test_an_enforced_pin_still_refuses_beside_a_noted_one(self):
        rc, _, st = self.run_stream(None, pin_args=["--sha256", "0" * 64, "--note-pin", hashlib.sha256(BODY).hexdigest()])
        self.assertEqual(rc, 3)
        self.assertTrue(st.startswith("STREAM_MISMATCH"), st)

    def test_an_enforced_pin_that_matches_passes_beside_a_wrong_noted_one(self):
        rc, out, st = self.run_stream(None, pin_args=["--sha256", hashlib.sha256(BODY).hexdigest(), "--note-pin", "0" * 64])
        self.assertEqual((rc, out), (0, BODY))
        self.assertIn("pin=differs", st)

    def test_stream_mismatch(self):
        rc, _, st = self.run_stream("0" * 64)
        self.assertEqual(rc, 3)
        self.assertTrue(st.startswith("STREAM_MISMATCH"))


if __name__ == "__main__":
    unittest.main()
