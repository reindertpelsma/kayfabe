"""Provider failures stay distinct from kayfabe test failures; no GPU or network."""

import io
import json
import os
import shlex
import tempfile
import time
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

import vast_payload as payload
import vast_report as reporter


class ProviderFaults(unittest.TestCase):
    def real_parent_receipt(self, rc, stopped=False, fragmented=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "bin"
            binary.mkdir()
            for name, text in {
                "dmesg": "#!/bin/sh\nprintf 'kernel fixture\\nkernel readable\\n'\n",
                "nvidia-smi": "#!/bin/sh\necho fixture-gpu\n",
            }.items():
                script = binary / name
                script.write_text(text)
                script.chmod(0o755)
            fixture = root / "fault.txt"
            fixture.write_text(self.real_case())
            marker = root / "xid-failed"
            provision = root / "provision_full.sh"
            provision.write_text(
                "#!/bin/bash\n"
                + ("printf 'DRIVER_RC=8\\n'\nsleep 0.15\n" if fragmented else "")
                + "cat "
                + shlex.quote(str(fixture))
                + "\n"
                + ("touch " + shlex.quote(str(marker)) + "\n" if stopped else "")
                + "exit "
                + str(rc)
                + "\n"
            )
            checkout = Path(payload.__file__).resolve().parents[2]
            command = payload.watched(["bash", str(provision)], checkout, "provider-fixture")
            bindings = {
                "PATH": str(binary) + os.pathsep + os.environ["PATH"],
                "XID_OUTDIR": str(root),
                "KF_PAYLOAD_XID_FAILED": str(marker),
            }
            with patch.dict(os.environ, bindings), patch.object(payload, "STOP", False):
                with redirect_stdout(io.StringIO()):
                    status = payload.run_bounded(
                        command, int(time.time() * 1000) + 10000, root / "output", []
                    )
            receipt = root / "xid_provider-fixture.log"
            return status, receipt.read_text() if receipt.exists() else ""

    def test_actual_parent_preserves_failed_watch_receipt_one(self):
        rc, text = self.real_parent_receipt(1)
        self.assertEqual(rc, 1)
        self.assertEqual(payload.watch_delta(text, rc, failed_provider=True), [])

    def test_actual_parent_preserves_failed_watch_receipt_two(self):
        rc, text = self.real_parent_receipt(2)
        self.assertEqual(rc, 2)
        self.assertEqual(payload.watch_delta(text, rc, failed_provider=True), [])

    def test_actual_parent_keeps_explicit_xid_stop_one(self):
        rc, text = self.real_parent_receipt(1, stopped=True)
        self.assertEqual(rc, 1)
        self.assertNotIn("HOST Xid CLEAN", text)

    def test_actual_parent_split_driver_receipt_one(self):
        rc, text = self.real_parent_receipt(1, fragmented=True)
        self.assertEqual(rc, 1)
        self.assertEqual(payload.watch_delta(text, rc, failed_provider=True), [])

    def test_actual_parent_split_driver_receipt_two(self):
        rc, text = self.real_parent_receipt(2, fragmented=True)
        self.assertEqual(rc, 2)
        self.assertEqual(payload.watch_delta(text, rc, failed_provider=True), [])

    def test_actual_parent_keeps_explicit_xid_stop_two(self):
        rc, text = self.real_parent_receipt(2, stopped=True)
        self.assertEqual(rc, 1)
        self.assertNotIn("HOST Xid CLEAN", text)

    def clean_failed_stage(self, rc):
        from test_vast_payload import watch

        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "result.json"
            watcher = Path(directory) / "watch.log"
            watcher.write_text(watch(rc=rc))

            def grade(status):
                self.assertEqual(status, rc)
                name = payload.provider_fault(self.real_case())
                if name:
                    raise payload.ProviderFault(name)
                raise payload.EvidenceError("provision")

            stage = {
                "name": "provision",
                "command": [],
                "output": target,
                "progress": [],
                "grade": lambda status: payload.stage_grade(None, None, watcher, grade, status),
            }
            result = {"verdict": "FAIL", "suites": [], "hardware": dict(reporter.UNKNOWN_HW)}
            with redirect_stdout(io.StringIO()):
                payload.execute_stages([stage], 10, result, target, runner=lambda *args: rc)
            self.assertEqual(result["verdict"], "PROVIDER_FAULT")
            self.assertEqual(result["suites"][0]["name"], "provider_gpu_unavailable")

    def test_clean_failed_provision_wrapper_one(self):
        self.clean_failed_stage(1)

    def test_clean_failed_provision_wrapper_two(self):
        self.clean_failed_stage(2)

    def mismatched_watch(self, command_rc, wrapper_rc):
        from test_vast_payload import watch

        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "watch.log"
            log.write_text(watch(rc=command_rc))

            def grade(rc):
                raise payload.ProviderFault("provider_gpu_unavailable")

            with self.assertRaises(payload.EvidenceError) as caught:
                payload.stage_grade(None, None, log, grade, wrapper_rc)
            self.assertNotIsInstance(caught.exception, payload.ProviderFault)

    def test_wrong_wrapper_exit_one(self):
        self.mismatched_watch(1, 2)

    def test_wrong_wrapper_exit_two(self):
        self.mismatched_watch(2, 1)

    def test_failed_command_is_not_product_pass_one(self):
        from test_vast_payload import watch

        with self.assertRaises(payload.EvidenceError):
            payload.watch_delta(watch(rc=1), 1)

    def test_failed_command_is_not_product_pass_two(self):
        from test_vast_payload import watch

        with self.assertRaises(payload.EvidenceError):
            payload.watch_delta(watch(rc=2), 2)

    def real_case(self):
        return (
            Path(__file__).resolve().parents[2]
            / "ci/vast-broker/test/fixtures/provider-57890737.txt"
        ).read_text()

    def test_real_3090_failed_swap_is_provider_fault(self):
        self.assertEqual(payload.provider_fault(self.real_case()), "provider_gpu_unavailable")

    def test_real_initialization_failure_is_not_a_product_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "result.json"

            def grade(rc):
                name = payload.provider_fault(self.real_case())
                if name:
                    raise payload.ProviderFault(name)
                raise payload.EvidenceError("provision")

            result = {"verdict": "FAIL", "suites": [], "hardware": dict(reporter.UNKNOWN_HW)}
            stage = {
                "name": "provision",
                "command": [],
                "output": target,
                "progress": [],
                "grade": grade,
            }
            with redirect_stdout(io.StringIO()):
                payload.execute_stages([stage], 10, result, target, runner=lambda *args: 1)
            self.assertEqual(result["verdict"], "PROVIDER_FAULT")

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

    def test_gpu_fault_flows_through_stage_result(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "result.json"
            log = "DRIVER_RC=8\nNOT_READY driver_rc=8 (GPU unusable)\nNVIDIA_SMI_RC=1\n"

            def grade(rc):
                name = payload.provider_fault(log)
                if name:
                    raise payload.ProviderFault(name)
                raise payload.EvidenceError("provision")

            result = {"verdict": "FAIL", "suites": [], "hardware": dict(reporter.UNKNOWN_HW)}
            stage = {
                "name": "provision",
                "command": [],
                "output": target,
                "progress": [],
                "grade": grade,
            }
            with redirect_stdout(io.StringIO()):
                payload.execute_stages([stage], 10, result, target, runner=lambda *args: 1)
            self.assertEqual(result["verdict"], "PROVIDER_FAULT")

    def test_missing_xid_proof_blocks_provider_fault(self):
        self.check_xid_guard("missing watcher evidence")

    def test_unmatched_xid_blocks_provider_fault(self):
        from test_vast_payload import watch, XID

        self.check_xid_guard(watch(XID, 0, 1, rc=1))

    def test_provider_watch_lost_baseline_stops(self):
        from test_vast_payload import watch

        self.check_xid_guard(watch(before=2, after=1, rc=1))

    def test_provider_watch_changed_counts_stops(self):
        from test_vast_payload import watch

        self.check_xid_guard(watch(before=1, after=2, rc=2), 2)

    def test_provider_watch_unwatched_exit_stops(self):
        from test_vast_payload import watch

        self.check_xid_guard(watch(rc=7), 7)

    def test_provider_watch_timeout_exit_stops(self):
        from test_vast_payload import watch

        self.check_xid_guard(watch(rc=124), 124)

    def check_xid_guard(self, text, rc=1):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "watch.log"
            log.write_text(text)

            def grade(rc):
                raise payload.ProviderFault("provider_gpu_unavailable")

            with self.assertRaises(payload.EvidenceError) as caught:
                payload.stage_grade(Path(directory), "test", log, grade, rc)
            self.assertNotIsInstance(caught.exception, payload.ProviderFault)

    def test_local_result_parser_accepts_explicit_provider_fault(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "result.json"
            target.write_text(
                json.dumps(
                    {
                        "verdict": "PROVIDER_FAULT",
                        "suites": [
                            {"name": "provider_kvm_missing", "pass": 0, "fail": 1, "skip": 0}
                        ],
                    }
                )
            )
            self.assertEqual(reporter.read_result_file(target)["verdict"], "PROVIDER_FAULT")


if __name__ == "__main__":
    unittest.main()
