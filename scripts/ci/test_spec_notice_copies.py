"""The three JS copies of the SPEC disclaimer equal the one in spec_stamp.py (docs/SPEC_RESULTS.md). GPU-free."""

import importlib.util
from pathlib import Path
import re
import unittest


class DisclaimerCopies(unittest.TestCase):
    def test_the_three_copies_of_the_disclaimer_are_the_one_in_spec_stamp(self):
        root = Path(__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location("spec_stamp", root / "scripts/bench/specviewperf/spec_stamp.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        for rel in ("ci/vast-broker/src/runreport.js", "ci/platform-api/src/subruns.js", "site/report_fixture.mjs"):
            src = (root / rel).read_text()
            i = src.index("SPEC_DISCLAIMER =")
            literal = src[i:src.index(";\n", i)]
            joined = "".join(re.findall(r"'([^']*)'", literal))
            self.assertEqual(joined, mod.DISCLAIMER, rel)


if __name__ == "__main__":
    unittest.main()
