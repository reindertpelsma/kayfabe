"""Thin guest negative controls must use S4's owned-process supervisor."""

import io
import json
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import vast_payload as payload

ROOT = Path(__file__).resolve().parents[2]
FIX = ROOT / "traces/sweep_windows_20261021/lease_57081143"


class GuestDelegation(unittest.TestCase):
    def invoke(self, arm, refused=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt = root / "native.json"
            baseline = json.loads((FIX / "native/native_xid_baseline.json").read_text())[
                "approved_native_xid_lines"
            ]
            extra = (FIX / "linux_stop/kernel_xids.log").read_text().splitlines()[5:]
            added = extra[:4] if arm == "--defer-liveness" else extra[4:5]
            receipt.write_text(json.dumps({"approved_native_xid_lines": baseline}))
            serial = root / "serial"
            serial.write_text("SERIAL_DIAGNOSTIC_EVIDENCE\n")
            qlog = root / "qemu.log"
            qlog.write_text("QEMU_DIAGNOSTIC_EVIDENCE\n")
            revision = "a" * 40
            binary = root / revision[:8] / "qemu-system-x86_64"
            arguments = [
                str(binary),
                "-name",
                "kf-fastguest",
                "-append",
                "console=hvc0 KF_ARMS=" + arm + ",--probe-launch-dma",
                "-chardev",
                "file,id=kfcon0,path=" + str(serial),
            ]
            calls = []

            def supervised(*args):
                calls.append(args)
                if refused:
                    raise payload.xid_guard.GuardRefused("unmatched source channel")
                receipt.write_text(
                    json.dumps(
                        {
                            "approved_native_xid_lines": baseline,
                            "accepted_xid_lines": baseline + added,
                        }
                    )
                )
                return 0

            env = {
                "KF_PAYLOAD_ROOT": str(ROOT),
                "KF_PAYLOAD_NATIVE_RECEIPT": str(receipt),
                "KF_PAYLOAD_LOG": str(root / "output"),
                "KF_PAYLOAD_DIAGNOSTIC_STATE": str(root / "diagnostics.json"),
                "KF_PAYLOAD_XID_ALLOWED": str(root / "allowed"),
                "KF_PAYLOAD_XID_FAILED": str(root / "failed"),
            }
            with patch.dict(os.environ, env), patch.object(
                payload.xid_guard, "supervise_guest", side_effect=supervised
            ), patch.object(
                payload.subprocess, "check_output", return_value=revision + "\n"
            ), redirect_stdout(
                io.StringIO()
            ):
                rc = payload.guest_command(qlog, arguments)
            self.assertEqual(len(calls), 1, "shared supervisor was bypassed")
            self.assertEqual(
                calls[0],
                (
                    binary,
                    arguments[1:],
                    receipt,
                    revision,
                    ROOT,
                    serial,
                    qlog,
                ),
            )
            output = (root / "output").read_text()
            allowed = (root / "allowed").read_text() if (root / "allowed").exists() else ""
            failed = (root / "failed").exists()
            return rc, output, allowed, failed, len(added)

    def test_defer_delegates_real_context_to_shared_supervisor(self):
        rc, _, allowed, failed, count = self.invoke("--defer-liveness")
        self.assertEqual(rc, 0)
        self.assertFalse(failed)
        self.assertEqual(len(allowed.splitlines()), count)

    def test_missing_page_delegates_real_context_to_shared_supervisor(self):
        rc, _, allowed, failed, count = self.invoke("--missing-page-fault")
        self.assertEqual(rc, 0)
        self.assertFalse(failed)
        self.assertEqual(len(allowed.splitlines()), count)

    def test_defer_refusal_stops_and_carries_serial_and_qemu_logs(self):
        rc, output, allowed, failed, _ = self.invoke("--defer-liveness", refused=True)
        self.assertNotEqual(rc, 0)
        self.assertTrue(failed)
        self.assertEqual(allowed, "")
        self.assertIn("SERIAL_DIAGNOSTIC_EVIDENCE", output)
        self.assertIn("QEMU_DIAGNOSTIC_EVIDENCE", output)


class GuestHook(unittest.TestCase):
    def invoke(self, arm):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            trace = root / "trace"
            qlog = root / "qemu.log"
            timer = root / "timeout"
            timer.write_text(
                "#!" + sys.executable + "\nimport os,json,sys\nfrom pathlib import Path\n"
                'Path(os.environ["TRACE"]+".timer").write_text(json.dumps(sys.argv[1:3]))\n'
                "os.execvp(sys.argv[3],sys.argv[3:])\n"
            )
            timer.chmod(0o700)
            python = root / "python3"
            python.write_text(
                "#!" + sys.executable + "\nimport os,json,sys\nfrom pathlib import Path\n"
                'Path(os.environ["TRACE"]).write_text(json.dumps(sys.argv))\n'
            )
            python.chmod(0o700)
            env = {
                **os.environ,
                "PATH": str(root) + ":" + os.environ["PATH"],
                "BASH_ENV": str(ROOT / "scripts/ci/vast_payload_hooks.sh"),
                "KF_PAYLOAD_ROOT": str(ROOT),
                "TRACE": str(trace),
                "QLOG": str(qlog),
            }
            command = (
                "timeout --kill-after=3 180 /fixture/qemu-system-x86_64 -name kf-fastguest "
                '-append "KF_ARMS=$1,--probe-launch-dma" > "$QLOG" 2>&1'
            )
            result = subprocess.run(
                ["/bin/bash", "-c", command, "fixture", arm],
                env=env,
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            args = json.loads(trace.read_text())
            self.assertEqual(
                json.loads(Path(str(trace) + ".timer").read_text()), ["--kill-after=3", "180"]
            )
            self.assertIn("--guest-exec", args)
            self.assertEqual(args[args.index("--guest-exec") + 1], str(qlog))

    def test_defer_hook_keeps_real_regular_log_and_original_timeout(self):
        self.invoke("--defer-liveness")

    def test_missing_hook_keeps_real_regular_log_and_original_timeout(self):
        self.invoke("--missing-page-fault")

    def test_missing_page_refusal_stops_and_carries_serial_and_qemu_logs(self):
        rc, output, allowed, failed, _ = self.invoke("--missing-page-fault", refused=True)
        self.assertNotEqual(rc, 0)
        self.assertTrue(failed)
        self.assertEqual(allowed, "")
        self.assertIn("SERIAL_DIAGNOSTIC_EVIDENCE", output)
        self.assertIn("QEMU_DIAGNOSTIC_EVIDENCE", output)
