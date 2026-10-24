"""B1g fix 03:30: a lease-preflight self-test ("dry loop") must pass before any create.

Run: KF_BW3_D=<attempt workspace> KF_BW3_K=<its run_kit> python3 -m unittest test_dry_loop
"""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

COMMON = Path("/workspace/bw3-round-b1g-common")
sys.path.insert(0, str(COMMON))
import dry_loop as DL  # noqa: E402

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1g-a2_m30498-run"))
K = Path(os.environ.get("KF_BW3_K", "/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/"
                        "ROUND_B1g_0050/attempt2_m30498/run_kit"))


class Receipt(unittest.TestCase):
    def roots(self, tmp):
        a = Path(tmp) / "a"
        a.mkdir()
        (a / "guard.py").write_text("def loop():\n    return 1\n")
        return [a]

    def test_missing_receipt_refuses(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(RuntimeError):
                DL.require_receipt(Path(tmp), roots=self.roots(tmp))

    def test_fresh_passing_receipt_accepts(self):
        with tempfile.TemporaryDirectory() as tmp:
            roots = self.roots(tmp)
            DL.write_receipt(Path(tmp), {"suites": [], "operator": {}, "static": []}, roots=roots)
            self.assertTrue(DL.require_receipt(Path(tmp), roots=roots)["passed"])

    def test_changed_source_after_receipt_refuses(self):
        with tempfile.TemporaryDirectory() as tmp:
            roots = self.roots(tmp)
            DL.write_receipt(Path(tmp), {"suites": [], "operator": {}, "static": []}, roots=roots)
            (roots[0] / "guard.py").write_text("def loop():\n    return new_old\n")
            with self.assertRaises(RuntimeError):
                DL.require_receipt(Path(tmp), roots=roots)

    def test_failed_suite_or_static_problem_refuses(self):
        with tempfile.TemporaryDirectory() as tmp:
            roots = self.roots(tmp)
            bad = [{"suite": "x", "ran": 1, "ok": False}]
            DL.write_receipt(Path(tmp), {"suites": bad, "operator": {}, "static": []}, roots=roots)
            with self.assertRaises(RuntimeError):
                DL.require_receipt(Path(tmp), roots=roots)
            DL.write_receipt(Path(tmp), {"suites": [], "operator": {},
                                         "static": [["f", 1, "undefined new_old"]]}, roots=roots)
            with self.assertRaises(RuntimeError):
                DL.require_receipt(Path(tmp), roots=roots)


    def test_added_source_file_after_receipt_refuses(self):
        with tempfile.TemporaryDirectory() as tmp:
            roots = self.roots(tmp)
            DL.write_receipt(Path(tmp), {"suites": [], "operator": {}, "static": []}, roots=roots)
            (roots[0] / "new_guard.py").write_text("x = 1\n")
            with self.assertRaises(RuntimeError):
                DL.require_receipt(Path(tmp), roots=roots)

    def test_failed_operator_dry_run_refuses(self):
        good = {"budget_watch": {"absence_receipt": True, "destroy_called": True},
                "precreate_watchdog": {"state": "ARMED", "result": "no contract before deadline"}}
        for broken in ({**good, "budget_watch": {"absence_receipt": False, "destroy_called": True}},
                       {**good, "precreate_watchdog": {"state": None, "result": None}}):
            with tempfile.TemporaryDirectory() as tmp:
                roots = self.roots(tmp)
                DL.write_receipt(Path(tmp), {"suites": [], "operator": broken, "static": []},
                                 roots=roots)
                with self.assertRaises(RuntimeError):
                    DL.require_receipt(Path(tmp), roots=roots)


    def receipt_for(self, body):
        with tempfile.TemporaryDirectory() as tmp:
            return DL.write_receipt(Path(tmp), body, roots=self.roots(tmp))

    def test_static_problem_marks_receipt_failed(self):
        body = {"suites": [], "operator": {}, "static": [["f", 1, "undefined x"]]}
        self.assertFalse(self.receipt_for(body)["passed"])

    def test_failed_suite_marks_receipt_failed(self):
        body = {"suites": [{"suite": "s", "ran": 3, "ok": False}], "operator": {}, "static": []}
        self.assertFalse(self.receipt_for(body)["passed"])

    def test_deleted_receipt_refuses(self):
        with tempfile.TemporaryDirectory() as tmp:
            roots = self.roots(tmp)
            DL.write_receipt(Path(tmp), {"suites": [], "operator": {}, "static": []}, roots=roots)
            (Path(tmp) / DL.RECEIPT).unlink()
            with self.assertRaises(RuntimeError):
                DL.require_receipt(Path(tmp), roots=roots)

    def test_budget_watch_dry_run_failure_marks_receipt_failed(self):
        op = {"budget_watch": {"absence_receipt": False, "destroy_called": True},
              "precreate_watchdog": {"state": "ARMED", "result": "no contract before deadline"}}
        self.assertFalse(self.receipt_for({"suites": [], "operator": op, "static": []})["passed"])

    def test_precreate_dry_run_failure_marks_receipt_failed(self):
        op = {"budget_watch": {"absence_receipt": True, "destroy_called": True},
              "precreate_watchdog": {"state": "ARMED", "result": "watchdog error: X"}}
        self.assertFalse(self.receipt_for({"suites": [], "operator": op, "static": []})["passed"])


class Loops(unittest.TestCase):
    def test_suite_list_covers_every_guard_loop(self):
        names = {s for _, s in DL.SUITES}
        for s in ("test_saved_xid32_watch_loop", "test_phase_watch_loop", "test_windows_observation",
                  "test_precreate_watchdog", "test_fuzz_findings", "test_native_findings_wrapper",
                  "test_media_wait", "test_init_observer", "test_console_pin_types"):
            self.assertIn(s, names)

    def test_operator_dry_runs_pass_with_fake_provider(self):
        result = DL.dry_operator(D)
        self.assertTrue(result["budget_watch"]["absence_receipt"], result)
        self.assertEqual(result["precreate_watchdog"]["result"], "no contract before deadline")
        self.assertEqual(result["precreate_watchdog"]["state"], "ARMED")

    def test_static_check_catches_an_injected_name_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "g.py"
            p.write_text("def loop(rows):\n    return new_old.kernel_rows(rows)\n")
            self.assertTrue(DL.static_problems([Path(tmp)]))


class Wiring(unittest.TestCase):
    def test_ops_requires_receipt_before_any_create(self):
        ops = (D / "ops.py").read_text()
        call = 'DL.require_receipt(D, roots=DL.default_roots(D, L.parent / "run_kit"))'
        self.assertIn(call, ops)
        self.assertLess(ops.index(call), ops.index('state = json.loads((D / "stretch.json")'))
        self.assertLess(ops.index(call), ops.index('"create",\n            "instance"'))
        self.assertLess(ops.index(call), ops.index("prewatch = subprocess.Popen("))

    def test_ops_gate_called_exactly_once_in_rent_branch(self):
        ops = (D / "ops.py").read_text()
        self.assertEqual(ops.count("DL.require_receipt("), 1)
        rent = ops.index('if mode in ("preflight", "rent"):')
        self.assertGreater(ops.index("DL.require_receipt("), rent)

    def test_receipt_present_and_current_for_this_workspace(self):
        receipt = DL.require_receipt(D, roots=DL.default_roots(D, K))
        self.assertTrue(receipt["passed"])
        self.assertGreater(sum(s["ran"] for s in receipt["suites"]), 100)


if __name__ == "__main__":
    unittest.main()
