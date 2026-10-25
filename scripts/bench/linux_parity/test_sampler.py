"""GPU-free sampler cadence and fail-closed query regression checks."""

import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch
import sampler


class SamplerTests(unittest.TestCase):
    def test_query_error_preserves_receipt_and_stops(self):
        sampler.running = True
        fake = types.SimpleNamespace(returncode=1, stdout="", stderr="unsupported-query")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "samples.jsonl"
            with patch.object(sampler.subprocess, "run", return_value=fake):
                with self.assertRaises(RuntimeError):
                    sampler.sample(path, 0.3)
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["rc"], 1)
            self.assertEqual(rows[0]["stderr"], "unsupported-query")

    def test_real_clock_query_brackets_and_cadence(self):
        sampler.running = True
        fake = types.SimpleNamespace(returncode=0, stdout="synthetic-only", stderr="")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "samples.jsonl"
            with patch.object(sampler.subprocess, "run", return_value=fake):
                sampler.sample(path, 0.36)
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertGreaterEqual(len(rows), 3)
            self.assertLessEqual(len(rows), 5)
            for row in rows:
                self.assertGreaterEqual(row["after"]["mono_ns"], row["before"]["mono_ns"])
            gaps = [b["before"]["mono_ns"] - a["before"]["mono_ns"] for a, b in zip(rows, rows[1:])]
            self.assertTrue(all(gap >= 90_000_000 for gap in gaps))


if __name__ == "__main__":
    unittest.main()
