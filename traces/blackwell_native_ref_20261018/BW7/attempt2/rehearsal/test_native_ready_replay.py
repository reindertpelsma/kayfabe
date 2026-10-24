"""Real native operator loop with a fake provider and strict-login tripwire."""

import datetime as dt
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from test_native_pin import A, B, X, C, K, Clock, native
import native_ready as R


class NativeReadyReplay(unittest.TestCase):
    def exercise(self, *, succeeds, template=R.TEMPLATE, native_lease=True):
        clock = Clock()
        born = dt.datetime(2026, 10, 24, tzinfo=dt.timezone.utc)
        calls = []
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "own-id").write_text("2")
            ledger = root / "ledger.json"
            row = dict(id=2, created_utc=born.isoformat(), auto_destroy_verified=True,
                       native_template_hash=template, no_kayfabe=native_lease,
                       guard_rate=0.5, destroyed=False)
            ledger.write_text(json.dumps([row]))
            status = dict(id=2, dph_total=0.4, public_ipaddr="8.8.8.8",
                          ports={"22/tcp": [{"HostPort": "2222"}]}, actual_status="running")
            def run(argv, **kwargs):
                calls.append((argv, clock.t))
                if argv[:3] == ["vastai", "show", "instances"]:
                    return subprocess.CompletedProcess(argv, 0, json.dumps([status]), "")
                if argv[:2] == ["vastai", "logs"]:
                    self.assertLessEqual(kwargs["timeout"], 20)
                    return subprocess.CompletedProcess(argv, 0, native(A, B), "")
                if argv[0] == "ssh-keyscan":
                    key = A if clock.t < 30 else (None if clock.t < 90 else X)
                    if succeeds and clock.t >= 180:
                        key = B
                    text = "host ssh-ed25519 " + key if key else ""
                    return subprocess.CompletedProcess(argv, 0, text, "")
                if argv[0] == "ssh":
                    proof_rows = [json.loads(s) for s in
                                  (root / "console_pin_2.jsonl").read_text().splitlines()]
                    proof = proof_rows[-1]
                    self.assertEqual(proof["latest"], C.fingerprint(B))
                    self.assertEqual(proof["scanned_fingerprint"], C.fingerprint(B))
                    self.assertGreaterEqual(proof["stable_seconds"], 120)
                    self.assertGreaterEqual(proof["consecutive"], 2)
                    self.assertIn("StrictHostKeyChecking yes", (root / "ssh.conf").read_text())
                    self.assertIn("User vast", (root / "ssh.conf").read_text())
                    self.assertIn(B, (root / "known_hosts").read_text())
                    self.assertLess(clock.t, 1200)
                    return subprocess.CompletedProcess(argv, 0, "CAPC_SSH_READY", "")
                raise AssertionError("unexpected operator command")
            error = None
            try:
                result = R.ready(root, ledger, lambda rows: ledger.write_text(json.dumps(rows)),
                                 clock=clock, sleep=clock.sleep, run=run,
                                 utc=lambda: born + dt.timedelta(seconds=clock.t))
            except (C.PinFlap, AssertionError) as exc:
                result, error = None, exc
            return result, error, calls, json.loads(ledger.read_text())[0], clock.t

    def test_staged_unknown_key_never_logs_in_then_windows_stabilizes(self):
        result, error, calls, row, end = self.exercise(succeeds=True)
        self.assertIsNone(error)
        self.assertEqual(result, 0)
        self.assertEqual(len([c for c in calls if c[0][0] == "ssh"]), 1)
        self.assertGreaterEqual(end, 300)
        self.assertEqual(row["ssh_hostkey_fingerprint"], C.fingerprint(B))

    def test_fixed_deadline_refuses_without_any_login(self):
        result, error, calls, row, end = self.exercise(succeeds=False)
        self.assertIsInstance(error, C.PinFlap)
        self.assertIsNone(result)
        self.assertFalse(any(c[0][0] == "ssh" for c in calls))
        self.assertNotIn("ssh_ready_utc", row)
        self.assertLessEqual(end, 1200)

    def test_other_paths_refuse_before_provider_or_scan(self):
        for template, native_lease in (("other", True), (R.TEMPLATE, False)):
            with self.subTest(template=template, native_lease=native_lease):
                result, error, calls, row, end = self.exercise(
                    succeeds=True, template=template, native_lease=native_lease)
                self.assertIsInstance(error, AssertionError)
                self.assertEqual(calls, [])
