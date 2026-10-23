"""V7 falsifiers: the PR104 result, immutable source and strict fault boundary."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

from test_vast_payload import MERGE, MARK, PROVISION, SHA, XID, watch


class V7Tests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "payload_v7", Path(__file__).with_name("vast_payload.py")
        )
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def test_only_the_marked_control_may_fault(self):
        self.assertFalse(self.m.xid_ok(watch(XID, 0, 1), 6, "--defer-liveness", MARK))

    def test_executor_control_gets_no_exemption(self):
        self.assertFalse(self.m.xid_ok(watch(XID, 0, 1), 6, "--executor-vas", MARK))

    def test_non_ce_client_is_rejected(self):
        bad = XID.replace("HUBCLIENT_CE0", "HUBCLIENT_GR")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_unknown_client_is_rejected(self):
        bad = XID.replace("HUBCLIENT_CE0", "")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_other_fault_type_is_rejected(self):
        bad = XID.replace("FAULT_PDE", "FAULT_PTE")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_read_fault_is_rejected(self):
        bad = XID.replace("VIRT_WRITE", "VIRT_READ")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_duplicate_marker_cannot_expand_allowance(self):
        text = watch(XID + "\n" + XID.replace("20.1", "20.2"), 0, 2)
        self.assertFalse(self.m.xid_ok(text, 6, "--missing-page-fault", MARK * 2))

    def test_marker_must_have_the_source_constant(self):
        wrong = MARK.replace("a11000000", "a12000000")
        bad = XID.replace("0xa_11000000", "0xa_12000000")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", wrong))

    def test_provision_conflicting_rc_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.provision_rows(PROVISION + "DRIVER_RC=1\n", 0)

    def test_provision_conflicting_build_rc_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.provision_rows(PROVISION + "KF3_RC=1\n", 0)

    def test_merge_conflicting_test_totals_refuse(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(MERGE + "TESTS passed 10 failed 2\n", 0, SHA)

    def test_merge_conflicting_gate_summary_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(MERGE + "V3_GATES_SUMMARY pass=8 fail=1 gate10=PASS\n", 0, SHA)

    def test_merge_prefix_survives_partial_evidence(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(MERGE.split("BARE_SUITE_PASS=")[0], 124, SHA)
        self.assertEqual(
            [r["name"] for r in caught.exception.rows], ["kf_tests", "gates", "kf3_build"]
        )

    def test_provision_head_is_bound_to_sha(self):
        source = Path(self.m.__file__).read_text()
        self.assertIn("prepare_checkout(root, sha)", source)
        self.assertIn("/workspace/kf-result.json", source)
        self.assertNotIn("/workspace/kf-ci-result.json", source)

    def test_result_has_only_the_pr104_payload_keys(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "result.json"
            result = {"verdict": "FAIL", "suites": [self.m.row("tests", 2)]}
            self.m.persist_result(path, result)
            self.assertEqual(json.loads(path.read_text()), result)
            self.assertFalse(path.with_suffix(".tmp").exists())

    def test_timeout_keeps_finished_prefix(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "result.json"
            result = {"verdict": "FAIL", "suites": [self.m.row("provision", 1)]}

            def grade(rc):
                raise self.m.EvidenceError("raw_suite", [self.m.row("kf_tests", 20)])

            stages = [
                {
                    "name": "merge",
                    "command": ["false"],
                    "output": path,
                    "progress": [],
                    "grade": grade,
                }
            ]
            rc = self.m.execute_stages(stages, 1, result, path, runner=lambda *args: 124)
            self.assertEqual(rc, 1)
            rows = json.loads(path.read_text())["suites"]
            self.assertEqual([r["name"] for r in rows], ["provision", "kf_tests", "timeout"])
            self.assertEqual(rows[-1]["fail"], 1)

    def test_first_bad_stage_cancels_later_stages(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "result.json"
            result = {"verdict": "FAIL", "suites": []}

            def grade(rc):
                raise self.m.EvidenceError("raw_suite")

            stages = [
                {
                    "name": "raw",
                    "command": ["false"],
                    "output": path,
                    "progress": [],
                    "grade": grade,
                },
                {
                    "name": "thin",
                    "command": ["true"],
                    "output": path,
                    "progress": [],
                    "grade": lambda rc: [self.m.row("thin", 30)],
                },
            ]
            calls = []
            rc = self.m.execute_stages(
                stages, 1, result, path, runner=lambda *a: calls.append(a) or 1
            )
            self.assertEqual(rc, 1)
            self.assertEqual(len(calls), 1)
            self.assertEqual(result["suites"], [self.m.row("raw_suite", failed=1)])

    def test_source_pin_uses_local_exact_approved_objects(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)

            def git(*args):
                return subprocess.check_output(["git", "-C", d, *args], text=True).strip()

            git("init", "-q")
            git("config", "user.email", "fixture@example.invalid")
            git("config", "user.name", "Fixture")
            git("commit", "-q", "--allow-empty", "-m", "fixture")
            sha = git("rev-parse", "HEAD")
            git("remote", "add", "origin", "https://invalid.example/repository")
            branch = self.m.prepare_checkout(root, sha)
            self.assertEqual(git("remote", "get-url", "origin"), d)
            self.assertEqual(git("rev-parse", "refs/heads/" + branch), sha)
            self.assertEqual(git("rev-parse", "refs/remotes/origin/" + branch), sha)

    def test_wrong_checkout_is_refused_before_remote_changes(self):
        with patch.object(self.m.subprocess, "check_output", return_value="b" * 40):
            with self.assertRaises(self.m.EvidenceError):
                self.m.prepare_checkout(Path("/fixture"), SHA)

    def test_red_cuda_row_is_not_progress(self):
        self.assertTrue(self.m.red_line("CL_ROW mode=host rung=cup3 rep=1 verdict=FAIL rc=1"))

    def test_red_crate_test_totals_are_not_progress(self):
        self.assertTrue(self.m.red_line("TESTS passed 90 failed 2"))

    def test_hook_failure_stops_owned_child(self):
        with tempfile.TemporaryDirectory() as d:
            failure = Path(d) / "xid-failed"
            code = "import time;time.sleep(20)"
            failure.touch()
            with patch.dict(os.environ, {"KF_PAYLOAD_XID_FAILED": str(failure)}):
                started = time.monotonic()
                rc = self.m.run_bounded(
                    ["python3", "-c", code], int(time.time() * 1000) + 30000, Path(d) / "log", []
                )
            self.assertNotEqual(rc, 0)
            self.assertLess(time.monotonic() - started, 3)


if __name__ == "__main__":
    unittest.main()
