"""One iteration of the native phase Xid watcher loop (phase_xid_guard main 'watch' mode).

B1g fix 03:30: every guard loop body runs under test with synthetic kernel input before a lease.
"""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import phase_xid_guard as G  # noqa: E402

PCI = "0000:00:07"
ACCEPTED = (
    "2026-10-24T00:57:03.284671+0000 ubuntu kernel: NVRM: Xid (PCI:0000:00:07): 31, pid=93202, "
    "name=kayfabe-rm-ladd, channel 0x00000002, intr 00000000. MMU Fault: ENGINE CE0 "
    "HUBCLIENT_CE2 faulted @ 0x60_00010000. Fault is of type FAULT_PTE ACCESS_TYPE_VIRT_READ"
)
OTHER = (
    "2026-10-24T01:10:00.000000+0000 ubuntu kernel: NVRM: Xid (PCI:0000:00:07): 32, pid=4444, "
    "name=qemu-system-x86, channel 0x00000015 intr 00200000"
)
BASE = "2026-10-24T00:50:00.000000+0000 ubuntu kernel: boot\n"


class Finished(BaseException):
    pass


class PhaseWatchLoop(unittest.TestCase):
    def run_loop(self, journal):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            receipt = root / "native_receipt.json"
            receipt.write_text(json.dumps({"approved_native_xid_lines": [ACCEPTED], "pci": PCI}))
            argv = ["phase_xid_guard.py", "watch", str(receipt), "1234", str(root / "phase.json"),
                    str(root / "fatal.json")]
            with (
                patch.object(sys, "argv", argv),
                patch.object(G.N, "kernel_snapshot", return_value=journal),
                patch.object(G.N, "terminate_owned_tree") as terminate,
                patch.object(G, "abort_owned_qemu", return_value=False),
                patch.object(G.os, "getpgid", return_value=1234),
                patch.object(G.os, "getpgrp", return_value=1234),
                patch.object(G.time, "sleep", side_effect=Finished),
            ):
                try:
                    outcome = G.main()
                except Finished:
                    outcome = "one_iteration"
            fatal = root / "fatal.json"
            accepted = Path(str(receipt) + ".accepted.json")
            return {
                "outcome": outcome,
                "fatal": json.loads(fatal.read_text()) if fatal.exists() else None,
                "accepted": json.loads(accepted.read_text()) if accepted.exists() else None,
                "terminate": terminate.call_count,
            }

    def test_first_iteration_with_only_accepted_history_keeps_running(self):
        r = self.run_loop(BASE + ACCEPTED + "\n")
        self.assertEqual(r["outcome"], "one_iteration", r["fatal"])
        self.assertIsNone(r["fatal"])
        self.assertEqual(r["accepted"]["approved_native_xid_lines"], [ACCEPTED])

    def test_new_xid_without_phase_stops_fail_closed(self):
        r = self.run_loop(BASE + ACCEPTED + "\n" + OTHER + "\n")
        self.assertEqual(r["outcome"], 7)
        self.assertIn("arm/init proof", r["fatal"]["STOP"])
        self.assertEqual(r["terminate"], 1)

    def test_lost_history_stops_fail_closed(self):
        r = self.run_loop(BASE + OTHER + "\n")
        self.assertEqual(r["outcome"], 7)
        self.assertIsNotNone(r["fatal"])

    def test_no_iteration_dies_on_a_name_error(self):
        for journal in (BASE + ACCEPTED + "\n", BASE + ACCEPTED + "\n" + OTHER + "\n"):
            r = self.run_loop(journal)
            self.assertNotIn("is not defined", json.dumps(r["fatal"] or {}))


if __name__ == "__main__":
    unittest.main()
