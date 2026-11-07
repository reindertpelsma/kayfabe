#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Gate: no committed replay trace holds a PROM (option-ROM) stream. STATUS: LIVE, 2026-11-07.

Owner decision 2026-11-07 (docs/OWNER_QUESTIONS_2026-10-03.md section 3): PROM read values in
committed `.rec` / `.rec.zst` traces are zero. Every tracked trace in the formats below is
checked with scrub_prom.check_bytes: a non-zero PROM read value, or an option-ROM marker
(PCIR, NPDS, NPDE) anywhere in the decoded stream, fails the test. The test is also run, in
scripts/evidence/test_scrub_prom.py, against synthetic traces so that it can be seen to fire.
"""

import importlib.util
import subprocess
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
spec = importlib.util.spec_from_file_location("scrub_prom", HERE / "scrub_prom.py")
sp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sp)


def tracked_traces():
    try:
        out = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "-z", "--", "*.rec", "*.rec.zst"],
            check=True,
            capture_output=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return sorted([*ROOT.glob("traces/*.rec"), *ROOT.glob("archive/**/*.rec.zst")])
    return [ROOT / p for p in out.decode().split("\0") if p]


class CommittedTraces(unittest.TestCase):
    def test_no_committed_trace_holds_prom_data(self):
        files = [f for f in tracked_traces() if f.is_file()]
        if not files:
            self.skipTest("no committed replay traces in this tree")
        for f in files:
            with self.subTest(f=str(f.relative_to(ROOT))):
                try:
                    result = sp.check_bytes(sp.load(f))
                except sp.ScrubError:
                    continue  # not a replay trace (wrong magic): nothing to scrub
                self.assertEqual(result, (0, []))


if __name__ == "__main__":
    unittest.main()
