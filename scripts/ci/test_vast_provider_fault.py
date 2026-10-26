"""Provider failures stay distinct from kayfabe test failures; no GPU or network."""

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

import vast_payload as payload
import vast_report as reporter


class ProviderFaults(unittest.TestCase):
    def test_missing_kvm_is_provider_fault(self):
        for name in ("missing-kvm", "missing-device"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                with self.assertRaises(payload.ProviderFault):
                    payload.require_provider_kvm(Path(directory) / name)

    def test_gpu_recovery_failure_has_two_bounded_signatures(self):
        for rc in (1, 124):
            with self.subTest(rc=rc):
                log = "DRIVER_RC=8\nNOT_READY driver_rc=8 (GPU unusable)\nNVIDIA_SMI_RC=%s\n" % rc
                self.assertEqual(payload.provider_fault(log), "provider_gpu_unavailable")

    def test_build_or_ambiguous_driver_failure_stays_fail(self):
        for log in (
            "DRIVER_RC=7\n",
            "DRIVER_RC=8\nNVIDIA_SMI_RC=0\n",
            "TREE_RC=1\n",
            "NOT_READY driver=unknown\n",
        ):
            self.assertIsNone(payload.provider_fault(log))

    def test_stage_persists_provider_verdict(self):
        for name in ("provider_kvm_missing", "provider_gpu_unavailable"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                target = Path(directory) / "result.json"

                def grade(rc):
                    raise payload.ProviderFault(name)

                result = {"verdict": "FAIL", "suites": [], "hardware": dict(reporter.UNKNOWN_HW)}
                stage = {
                    "name": "provision",
                    "command": [],
                    "output": target,
                    "progress": [],
                    "grade": grade,
                }
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(
                        payload.execute_stages([stage], 10, result, target, runner=lambda *args: 1),
                        1,
                    )
                self.assertEqual(json.loads(target.read_text())["verdict"], "PROVIDER_FAULT")

    def test_reporter_keeps_provider_verdict_on_nonzero_exit(self):
        for name in ("provider_kvm_missing", "provider_gpu_unavailable"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                target = Path(directory) / "result.json"
                target.write_text(
                    json.dumps(
                        {
                            "verdict": "PROVIDER_FAULT",
                            "suites": [{"name": name, "pass": 0, "fail": 1, "skip": 0}],
                        }
                    )
                )
                env = {"KF_NONCE": "b" * 32, "KF_SHA": "a" * 40, "KF_RUNNER_NONCE": "f" * 64}
                result = reporter.final_result(env, 1, False, None, target)
                self.assertEqual(result["verdict"], "PROVIDER_FAULT")


if __name__ == "__main__":
    unittest.main()
