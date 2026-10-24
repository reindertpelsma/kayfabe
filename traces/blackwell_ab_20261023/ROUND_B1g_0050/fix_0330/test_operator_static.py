"""B1g fix 03:30: the operator workspace, run kit and harness pass the static checks.

Run: KF_BW3_D=<attempt workspace> KF_BW3_K=<its run_kit> python3 -m unittest test_operator_static
"""

import os
from pathlib import Path
import sys
import unittest

COMMON = Path("/workspace/bw3-round-b1g-common")
sys.path.insert(0, str(COMMON))
import static_check as S  # noqa: E402

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1g-a1_m30498-run"))
K = Path(os.environ.get("KF_BW3_K", "/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/"
                        "ROUND_B1g_0050/attempt1_m30498/run_kit"))


def files(*roots):
    return [p for r in roots for p in sorted(Path(r).glob("*.py")) if not p.name.startswith("test_")]


class Static(unittest.TestCase):
    def test_harness_guards(self):
        self.assertEqual(S.check(files(*S.ROOTS)), [])

    def test_operator_workspace(self):
        self.assertEqual(S.check(files(D)), [])

    def test_run_kit(self):
        self.assertEqual(S.check(files(K)), [])

    def test_final_remote_audit_resolves_every_observation_attribute(self):
        self.assertEqual(S.attribute_problems(D / "final_remote_audit.py"), [])

    def test_checker_still_detects_the_original_defect(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "probe.py"
            p.write_text("import os\n\ndef loop(rows):\n    return new_old.kernel_rows(rows)\n")
            self.assertEqual([r[2] for r in S.check([p])], ["undefined new_old"])

    def test_checker_detects_missing_module_attribute(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "probe.py"
            p.write_text("import saved_xid32_observation as O\nO.check_host_health('x')\n")
            self.assertEqual([r[2] for r in S.check([p])],
                             ["missing attribute O.check_host_health"])


if __name__ == "__main__":
    unittest.main()
