"""Per-arm diagnostics must survive a rental without SSH access."""

import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import vast_payload as payload
from test_vast_payload import XID, watch

HERE = Path(__file__).resolve().parent


class ArmDispatch(unittest.TestCase):
    def selected_arm(self, arm):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bash = root / "bash"
            bash.write_text("#!/bin/sh\nexit 0\n")
            bash.chmod(0o700)
            python = root / "python3"
            python.write_text(
                "#!" + sys.executable + "\nimport json,os,sys\nfrom pathlib import Path\n"
                'Path(os.environ["TRACE"]).write_text(json.dumps(sys.argv))\n'
            )
            python.chmod(0o700)
            env = {
                **os.environ,
                "PATH": str(root) + ":" + os.environ["PATH"],
                "BASH_ENV": str(HERE / "vast_payload_hooks.sh"),
                "KF_PAYLOAD_ROOT": str(HERE.parents[1]),
                "BENCH_DIR": str(root),
                "TRACE": str(root / "trace"),
            }
            result = subprocess.run(
                [
                    "/bin/bash",
                    "-c",
                    'timeout --kill-after=5 180 /fixture/kayfabe-rm-ladder "$1"',
                    "fixture",
                    arm,
                ],
                env=env,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            args = json.loads((root / "trace").read_text())
            return args[args.index("--grade-xid") + 1]

    def test_defer_arm_is_selected_after_timeout_options(self):
        self.assertEqual(self.selected_arm("--defer-liveness"), "--defer-liveness")

    def test_executor_arm_is_selected_after_timeout_options(self):
        self.assertEqual(self.selected_arm("--executor-vas"), "--executor-vas")

    def test_missing_page_arm_still_selects_its_own_name(self):
        self.assertEqual(self.selected_arm("--missing-page-fault"), "--missing-page-fault")


class ArmDiagnostics(unittest.TestCase):
    def invoke(self, fault=False, body="diagnostic evidence\n"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "watch"
            text = (
                watch(XID, 0, 1)
                if fault
                else watch("", 0, 0).replace("\nfixture\n", "\n" + body + "\n", 1)
            )
            log.write_text(text)
            env = {
                "KF_PAYLOAD_LOG": str(root / "output"),
                "KF_PAYLOAD_DIAGNOSTIC_STATE": str(root / "diagnostics.json"),
                "KF_PAYLOAD_XID_ALLOWED": str(root / "allowed"),
                "KF_PAYLOAD_XID_FAILED": str(root / "failed"),
            }
            with patch.dict(os.environ, env), redirect_stdout(io.StringIO()):
                rc = payload.guard_command("--timer", log, 6 if fault else 0)
            self.assertTrue(
                (root / "output").is_file(), "per-arm evidence never reached callback log"
            )
            output = (root / "output").read_text()
            return rc, output

    def test_clean_arm_diagnostics_are_copied_to_callback_log(self):
        rc, output = self.invoke()
        self.assertEqual(rc, 0)
        self.assertIn("diagnostic evidence", output)
        self.assertIn("arm=--timer", output)

    def test_refused_arm_diagnostics_are_copied_before_failure(self):
        rc, output = self.invoke(fault=True)
        self.assertNotEqual(rc, 0)
        self.assertIn("NVRM", output)
        self.assertIn("arm=--timer", output)

    def test_large_arm_diagnostics_are_bounded(self):
        _, output = self.invoke(body="HEAD_EVIDENCE\n" + "x" * 150000 + "\nTAIL_EVIDENCE\n")
        self.assertLess(len(output.encode()), 40000)
        self.assertIn("truncated", output)

    def test_large_arm_diagnostics_keep_head_and_tail(self):
        _, output = self.invoke(body="HEAD_EVIDENCE\n" + "x" * 150000 + "\nTAIL_EVIDENCE\n")
        self.assertIn("HEAD_EVIDENCE", output)
        self.assertIn("TAIL_EVIDENCE", output)

    def failed_result(self, arm):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "diagnostics.json"
            state.write_text(
                json.dumps({"bytes": 0, "events": [{"arm": arm, "rc": 6, "admitted": False}]})
            )
            result = {"verdict": "FAIL", "suites": []}

            def failed(_):
                raise payload.EvidenceError("raw_suite")

            stage = {
                "name": "raw",
                "command": [],
                "output": root / "out",
                "progress": [],
                "grade": failed,
            }
            with patch.dict(
                os.environ, {"KF_PAYLOAD_DIAGNOSTIC_STATE": str(state)}
            ), redirect_stdout(io.StringIO()):
                payload.execute_stages(
                    [stage],
                    int(time.time() * 1000) + 5000,
                    result,
                    root / "result",
                    runner=lambda *_: 1,
                )
            return result

    def test_failed_defer_arm_has_a_structured_suite_row(self):
        self.assertIn(
            {"name": "arm_defer_liveness", "pass": 0, "fail": 1, "skip": 0},
            self.failed_result("--defer-liveness")["suites"],
        )

    def test_failed_missing_page_arm_has_a_structured_suite_row(self):
        self.assertIn(
            {"name": "arm_missing_page_fault", "pass": 0, "fail": 1, "skip": 0},
            self.failed_result("--missing-page-fault")["suites"],
        )
