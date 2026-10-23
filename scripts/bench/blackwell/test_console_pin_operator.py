"""Execute the real rental readiness branch with fake provider, SSH and lease clocks."""

import base64
import datetime as dt
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from test_console_pin import C, FP, KEY, OTHER

KIT = Path(__file__).resolve().parents[3] / "traces/blackwell_ab_20261023/observation_run/run_kit"


class OperatorConsolePin(unittest.TestCase):
    def run_ready(self, snapshots, elapsed=600, factor=4):
        self.assertIsNotNone(C, "console publication helper missing")
        clock = [float(elapsed)]
        calls = []
        born = dt.datetime(2026, 10, 23, tzinfo=dt.timezone.utc)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ledger = root / "owned_instances.json"
            row = {
                "id": 1234,
                "machine_id": 42,
                "auto_destroy_verified": True,
                "auto_destroy_pid": os.getpid(),
                "created_utc": born.isoformat(),
                "prior_lane_runtime_usd": 5.2,
                "budget_cutoff_minutes": 150,
            }
            ledger.write_text(json.dumps([row]))
            (root / "own-id").write_text("1234")
            status = {
                "id": 1234,
                "public_ipaddr": "8.8.8.8",
                "ports": {"22/tcp": [{"HostPort": "2222"}]},
                "actual_status": "running",
                "intended_status": "running",
                "dph_total": 0.3,
            }
            console_calls = [0]

            def run(argv, **kwargs):
                calls.append((tuple(argv[:2]), clock[0]))
                if argv[:2] == ["vastai", "logs"]:
                    index = console_calls[0]
                    console_calls[0] += 1
                    self.assertFalse((root / "known_hosts").exists())
                    text = snapshots[min(index, len(snapshots) - 1)]
                    return subprocess.CompletedProcess(argv, 0, text, "")
                if argv[0] == "ssh-keyscan":
                    clock[0] += 0.1
                    return subprocess.CompletedProcess(argv, 0, "host ssh-ed25519 " + KEY, "")
                if argv[0] == "ssh":
                    self.assertIn("StrictHostKeyChecking yes", (root / "ssh.conf").read_text())
                    self.assertIn(KEY, (root / "known_hosts").read_text())
                    return subprocess.CompletedProcess(argv, 0, "CAPC_SSH_READY", "")
                raise AssertionError("unexpected command")

            def failure(value, age, *, banner_seconds=None):
                bound = 1200 if banner_seconds is None else 600
                return "provider_boot_timeout" if age >= bound else None

            def save(rows):
                ledger.write_text(json.dumps(rows))

            fake_subprocess = SimpleNamespace(
                run=run,
                TimeoutExpired=subprocess.TimeoutExpired,
                CompletedProcess=subprocess.CompletedProcess,
            )
            namespace = dict(
                D=root,
                L=ledger,
                Path=Path,
                json=json,
                os=os,
                sys=sys,
                now=lambda: born + dt.timedelta(seconds=clock[0]),
                api=lambda args: [status],
                save=save,
                C=C,
                ipaddress=ipaddress,
                base64=base64,
                hashlib=hashlib,
                subprocess=fake_subprocess,
                time=SimpleNamespace(
                    monotonic=lambda: clock[0],
                    sleep=lambda seconds: clock.__setitem__(0, clock[0] + seconds),
                ),
                P=SimpleNamespace(
                    failure=failure,
                    boot_bound=lambda value: 1200 if value is None else 600,
                    policy=lambda seconds: {"factor": factor},
                ),
            )
            source = (KIT / "operator_ops.py").read_text()
            body = source.split('elif mode == "ready":', 1)[1].split('elif mode == "destroy":', 1)[
                0
            ]
            with patch.object(os, "kill"), self.assertRaises(SystemExit) as exit_code:
                exec("if True:\n" + body, namespace)
            proof = root / "provider_failure_1234.json"
            refusal = json.loads(proof.read_text()) if proof.exists() else None
            return exit_code.exception.code, calls, refusal

    def test_absent_console_then_present_after_ten_minutes_becomes_readiness(self):
        code, calls, refusal = self.run_ready(["booting", FP], elapsed=610)
        self.assertEqual(code, 0)
        self.assertEqual(len([c for c in calls if c[0] == ("vastai", "logs")]), 2)
        self.assertIsNone(refusal)

    def test_late_matching_pin_does_not_shorten_the_create_ceiling(self):
        code, calls, refusal = self.run_ready([FP], elapsed=1000, factor=10)
        self.assertEqual(code, 0)
        self.assertIsNone(refusal)

    def test_absent_pin_past_create_bound_is_a_readiness_timeout(self):
        code, calls, refusal = self.run_ready(["booting"], elapsed=1190, factor=10)
        self.assertEqual(code, 76)
        self.assertEqual(refusal["reason"], "provider_boot_timeout")
        self.assertFalse(any(c[0][0] == "ssh" for c in calls))

    def test_published_different_pin_stops_without_any_ssh_connection(self):
        with self.assertRaises(C.PinMismatch):
            self.run_ready(["ssh-ed25519 " + OTHER], elapsed=610)


if __name__ == "__main__":
    unittest.main()
