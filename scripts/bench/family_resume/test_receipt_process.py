"""Actual independent fake watcher survives creator exit, without provider access."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest


class ProcessTests(unittest.TestCase):
    def fixture(self, mode):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "case"
            try:
                result = subprocess.run(
                    [
                        sys.executable,
                        str(Path(__file__).with_name("receipt_process_fixture.py")),
                        "creator",
                        str(root),
                        mode,
                    ],
                    capture_output=True,
                    timeout=6,
                )
                self.assertIn(result.returncode, [17, 23], result.stderr.decode())
                until = time.monotonic() + 4
                while not (root / "watch_done.json").exists():
                    self.assertLess(time.monotonic(), until, "independent fake watcher failed")
                    time.sleep(0.01)
                done = json.loads((root / "watch_done.json").read_text())
                self.assertEqual(done["id"], 42)
                self.assertTrue(done["capture_verified"])
                self.assertTrue(done["absence_verified"])
                self.assertEqual(json.loads((root / "owned.json").read_text())[0]["id"], 42)
                events = [
                    json.loads(l)["kind"] for l in (root / "events.jsonl").read_text().splitlines()
                ]
                self.assertEqual(events.count("create"), 1)
                self.assertLess(events.index("intent_durable"), events.index("ARMED"))
                self.assertLess(events.index("ARMED"), events.index("create"))
                self.assertLess(events.index("capture42"), events.index("destroy42"))
                for name in ["create_outcome.json", "owned.json", "watch_done.json"]:
                    p = root / name
                    if p.exists():
                        self.assertNotIn("FAKE_CREATE_SECRET_MUST_NEVER_PERSIST", p.read_text())
            finally:
                p = root / "watch_process.json"
                if p.exists():
                    proc = json.loads(p.read_text())
                    stat = Path("/proc", str(proc["pid"]), "stat")
                    try:
                        born = int(stat.read_text().rpartition(")")[2].split()[19])
                    except (FileNotFoundError, ProcessLookupError):
                        born = None
                    if born == proc["starttime"]:
                        try:
                            os.killpg(proc["pid"], signal.SIGTERM)
                        except ProcessLookupError:
                            pass

    def test_empty_response_parent_exit_independent_watch(self):
        self.fixture("empty")

    def test_timeout_parent_exit_independent_watch(self):
        self.fixture("timeout")

    def test_parent_dies_before_receipt_or_ledger_publish(self):
        self.fixture("crash")


if __name__ == "__main__":
    unittest.main()
