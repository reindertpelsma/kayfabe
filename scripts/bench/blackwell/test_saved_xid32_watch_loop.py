"""One watcher-loop iteration of the saved-Xid32 OBSERVATION mode with synthetic kernel input.

MEASURED lease 57443671 (2026-10-24): the loop body had never run under test and stopped
fail-closed on its first iteration with NameError `new_old`. These tests run the real loop.
"""

import datetime as dt
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import test_windows_observation as base

SPEC = importlib.util.spec_from_file_location(
    "saved_xid32_observation_loop", Path(__file__).with_name("saved_xid32_observation.py")
)
O = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(O)

MODE = "bw3-saved-xid32-observation-v1"
AUTHORITY = {
    **base.AUTHORITY,
    "mode": MODE,
    "observation_only": True,
    "attestation_allowed": False,
    "exact_channels": ["0x14", "0x1000000b"],
}
PERMIT = {**base.PERMIT, "mode": MODE}
INITIAL = [
    {"cursor": "a", "priority": 6, "line": "2026-10-23T11:58:00.000000+0000 ubuntu kernel: one"},
    {"cursor": "b", "priority": 6, "line": "2026-10-23T11:58:01.000000+0000 ubuntu kernel: two"},
]


class Finished(BaseException):
    """Ends the loop after one iteration without entering the guard's Exception handler."""


def pair(count=2):
    return [base.xid(0x14 if n % 2 == 0 else 0x1000000B, second=n + 1) for n in range(count)]


def rows(lines):
    return INITIAL + [
        {"cursor": "x%d" % i, "priority": 3, "line": line} for i, line in enumerate(lines)
    ]


class Clock(dt.datetime):
    @classmethod
    def now(cls, tz=None):
        return base.NOW


class SavedWatchLoop(unittest.TestCase):
    def run_loop(self, current, *, phase=True, permit=True, initial=INITIAL):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = [
                ("native.json", {"approved_native_xid_lines": [], "pci": base.PHASE["pci"]}),
                ("authority.json", AUTHORITY),
            ]
            if phase:
                files.append(("phase.json", base.PHASE))
            if permit:
                files.append(("permit.json", PERMIT))
            for name, value in files:
                (root / name).write_text(json.dumps(value))
            args = [
                str(root / "native.json"),
                1234,
                str(root / "phase.json"),
                str(root / "fatal.json"),
                str(root / "authority.json"),
                str(root / "permit.json"),
            ]
            with (
                patch.object(O.old, "kernel_rows", side_effect=[initial, current]),
                patch.object(O.old, "check_host_health", return_value={"devices": 1}),
                patch.object(O.G, "validate_phase_owner", return_value=True),
                patch.object(O.G, "abort_owned_qemu", return_value=True) as abort,
                patch.object(O.G.N, "terminate_owned_tree") as terminate,
                patch.object(O, "dmesg_snapshot", return_value="\n".join(r["line"] for r in current)),
                patch.object(O.os, "getpgid", return_value=1234),
                patch.object(O.os, "getpgrp", return_value=1234),
                patch.object(O.dt, "datetime", Clock),
                patch.object(O.time, "sleep", side_effect=Finished),
            ):
                try:
                    O.watch(*args)
                    outcome = "returned"
                except Finished:
                    outcome = "one_iteration"
                except Exception as error:  # the guard re-raises after recording the stop
                    outcome = error
            fatal = root / "fatal.json"
            accepted = root / "native.json.accepted.json"
            return {
                "outcome": outcome,
                "fatal": json.loads(fatal.read_text()) if fatal.exists() else None,
                "accepted": json.loads(accepted.read_text()) if accepted.exists() else None,
                "abort": abort.call_count,
                "terminate": terminate.call_count,
                "context": sorted(p.name for p in root.rglob("*_observation_context.json")),
            }

    def test_first_iteration_without_phase_or_new_xid_keeps_running(self):
        r = self.run_loop(INITIAL, phase=False, permit=False)
        self.assertEqual(r["outcome"], "one_iteration", r["fatal"])
        self.assertIsNone(r["fatal"])
        self.assertEqual(r["accepted"]["observations"], {})

    def test_owned_phase_without_new_xid_keeps_running(self):
        r = self.run_loop(INITIAL)
        self.assertEqual(r["outcome"], "one_iteration", r["fatal"])
        self.assertIsNone(r["fatal"])
        self.assertEqual(r["abort"], 0)

    def test_new_admitted_pair_is_recorded_as_observation(self):
        r = self.run_loop(rows(pair()))
        self.assertEqual(r["outcome"], "one_iteration", r["fatal"])
        self.assertIsNone(r["fatal"])
        records = r["accepted"]["observations"]["on_observe_b"]
        self.assertEqual([x["channel"] for x in records], ["0x14", "0x1000000b"])
        self.assertTrue(all(x["verdict"] == "OBSERVATION" and not x["pass"] for x in records))
        self.assertEqual(len(r["context"]), 1)

    def test_new_non_admitted_xid_stops_fail_closed(self):
        r = self.run_loop(rows([base.xid(0x15)]))
        self.assertIsInstance(r["outcome"], O.Refused)
        self.assertIn("channel", r["fatal"]["STOP"])
        self.assertEqual((r["abort"], r["terminate"]), (1, 1))

    def test_new_other_code_xid_stops_fail_closed(self):
        r = self.run_loop(rows([base.xid(code=31)]))
        self.assertIsInstance(r["outcome"], O.Refused)
        self.assertEqual((r["abort"], r["terminate"]), (1, 1))

    def test_ninth_xid_stops_fail_closed(self):
        r = self.run_loop(rows(pair(9)))
        self.assertIsInstance(r["outcome"], O.Refused)
        self.assertIn("eight", r["fatal"]["STOP"])
        self.assertEqual((r["abort"], r["terminate"]), (1, 1))

    def test_new_xid_without_permit_stops_fail_closed(self):
        r = self.run_loop(rows(pair()), permit=False)
        self.assertIsInstance(r["outcome"], Exception)
        self.assertIsNotNone(r["fatal"])
        self.assertNotIn("is not defined", r["fatal"]["STOP"])

    def test_new_kernel_error_row_stops_fail_closed(self):
        error = {"cursor": "e", "priority": 2,
                 "line": "2026-10-23T11:59:30.000000+0000 ubuntu kernel: NVRM: GPU has fallen off"}
        r = self.run_loop(INITIAL + [error])
        self.assertIsInstance(r["outcome"], O.Refused)
        self.assertIn("kernel error", r["fatal"]["STOP"])

    def test_pre_existing_error_row_is_not_judged_again(self):
        old_error = {"cursor": "e0", "priority": 3,
                     "line": "2026-10-23T11:57:00.000000+0000 ubuntu kernel: earlier warning"}
        history = INITIAL + [old_error]
        r = self.run_loop(history, initial=history)
        self.assertEqual(r["outcome"], "one_iteration", r["fatal"])
        self.assertIsNone(r["fatal"])

    def test_replaced_kernel_history_stops_fail_closed(self):
        replaced = [{"cursor": "z", "priority": 6, "line": INITIAL[0]["line"]}, INITIAL[1]]
        r = self.run_loop(replaced)
        self.assertIsInstance(r["outcome"], Exception)
        self.assertIsNotNone(r["fatal"])

    def test_no_iteration_dies_on_a_name_error(self):
        for current in (INITIAL, rows(pair()), rows([base.xid(0x15)])):
            r = self.run_loop(current)
            self.assertNotIsInstance(r["outcome"], NameError, r["fatal"])


if __name__ == "__main__":
    unittest.main()
