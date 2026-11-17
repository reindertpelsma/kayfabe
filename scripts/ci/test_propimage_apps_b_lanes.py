#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free: discovery-lane rows boot alone (a crash or hang of one row cannot take the next one with it); gate rows share boots. STATUS: LIVE, 2026-11-16."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import propimage_apps_b_stage as S  # noqa: E402


class T(unittest.TestCase):
    def test_discovery_rows_are_alone(self):
        order = ["g1", "g2", "d1", "g3", "d2", "d3", "g4"]
        lanes = {"g1": "gate", "g2": "gate", "g3": "gate", "g4": "gate", "d1": "discovery", "d2": "discovery", "d3": "discovery"}
        boots = S.plan_boots({a: 100 for a in order}, order, lanes=lanes)
        self.assertEqual(boots, [["g1", "g2"], ["d1"], ["g3"], ["d2"], ["d3"], ["g4"]])

    def test_no_lanes_is_the_old_greedy_plan(self):
        order = ["a", "b", "c"]
        self.assertEqual(S.plan_boots({a: 10 for a in order}, order), [["a", "b", "c"]])


class Ref(unittest.TestCase):
    def test_reference_compare(self):
        import json, os, tempfile, zlib, struct
        sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench/windows"))
        import propimage_apps_b as B

        def png(path, v):
            raw = b"".join(b"\x00" + bytes([v, (v * 7) % 256, (v * 13) % 256]) * 8 for _ in range(8))
            def chunk(t, b):
                return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b))
            path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 8, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))

        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            d = td / "row"; d.mkdir(); ref = td / "ref"; ref.mkdir()
            png(d / "out.png", 100)
            row = {"ok": True}
            B._record_output_stats("r", d, row)
            stats = json.loads((d / "output_stats.json").read_text())
            self.assertEqual(row["outputs_recorded"], 1)
            (ref / "r.json").write_text(json.dumps(stats))
            os.environ["KF_REFERENCE_DIR"] = str(ref)
            try:
                row = {"ok": True}
                B._record_output_stats("r", d, row)
                self.assertTrue(row["reference"]["match"])
                png(d / "out.png", 180)
                row = {"ok": True}
                B._record_output_stats("r", d, row)
                self.assertFalse(row["reference"]["match"])
                self.assertTrue(row["ok"])          # not strict: recorded only
                os.environ["KF_REFERENCE_STRICT"] = "1"
                B._record_output_stats("r", d, row)
                self.assertFalse(row["ok"])
            finally:
                os.environ.pop("KF_REFERENCE_DIR", None); os.environ.pop("KF_REFERENCE_STRICT", None)


if __name__ == "__main__":
    unittest.main()
