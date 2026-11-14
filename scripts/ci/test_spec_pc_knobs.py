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

    def run_stream(self, pin):
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory() as d:
                uf = Path(d) / "u.json"
                uf.write_text(json.dumps({"k": "http://127.0.0.1:%d/o" % srv.server_port}))
                st = Path(d) / "st"
                p = subprocess.run([sys.executable, "-I", str(SPEC / "stream_url.py"), "--url-file", str(uf), "--key", "k",
                                    "--sha256", pin, "--status", str(st)], capture_output=True, timeout=60)
                return p.returncode, p.stdout, st.read_text()
        finally:
            srv.shutdown()

    def test_stream_match(self):
        rc, out, st = self.run_stream(hashlib.sha256(BODY).hexdigest())
        self.assertEqual((rc, out), (0, BODY))
        self.assertTrue(st.startswith("STREAM_OK"))

    def test_stream_mismatch(self):
        rc, _, st = self.run_stream("0" * 64)
        self.assertEqual(rc, 3)
        self.assertTrue(st.startswith("STREAM_MISMATCH"))


if __name__ == "__main__":
    unittest.main()
