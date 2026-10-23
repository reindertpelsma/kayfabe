"""A bounded missing CIM sample is never evidence of disabled or enabled state."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import init_observer as O
import phase_xid_guard as G


def row(code=0, tag="actual"):
    return "BW580_RESULT " + json.dumps({
        "code": code, "driver": "32.0.15.8088", "tdr4101": 0, "sample": tag,
    }) + "\n"


def missing(text=None):
    return subprocess.TimeoutExpired(["fixture-cim"], 10, output=text)


class MissingSamples(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.phase = Path(self.temp.name) / "phase.json"
        self.owner = {
            "arm": "on_precision", "pid": os.getpid(), "starttime": 123,
            "exe": "/own/qemu", "pci": "0000:00:07",
        }
        self.phase.write_text(json.dumps(self.owner))
        self.events = []
        self.clock = 0

    def run_samples(self, samples, *, attempts=None, change=None, pause=None):
        pending = iter(samples)

        def fetch():
            self.events.append("fetch")
            item = next(pending)
            if isinstance(item, BaseException):
                if change:
                    self.phase.write_text(json.dumps(dict(self.owner, **change)))
                raise item
            if callable(item):
                item = item()
            return item

        def ready():
            self.events.append("ready")

        def record(text):
            self.events.append(("record", json.loads(text.split(" ", 1)[1])))

        return O.observe(
            self.phase, "on_precision", fetch, ready, record, lambda _: True,
            lambda: self.clock < 120, attempts=attempts or len(samples),
            pause=pause or (lambda: None),
        )

    def test_first_timeout_waits_for_an_actual_disabled_sample(self):
        self.assertTrue(self.run_samples([missing(), row(22), row()]))
        self.assertEqual(self.events[:4], ["fetch", "fetch", "ready", "fetch"])

    def test_timeout_between_handshake_and_code0_keeps_same_owner_proof(self):
        self.assertTrue(self.run_samples([row(22), missing(), row()]))
        self.assertEqual(self.events.count("ready"), 1)
        self.assertEqual(self.events[-1][0], "record")

    def test_timeout_does_not_reset_an_existing_handshake(self):
        self.assertTrue(self.run_samples([row(22), missing(), row(22), row()]))
        self.assertEqual(self.events.count("ready"), 1)

    def test_code0_after_a_missing_first_sample_is_not_a_handshake(self):
        with self.assertRaisesRegex(ValueError, "disabled"):
            self.run_samples([missing(), row()])
        self.assertNotIn("ready", self.events)

    def test_code43_after_a_missing_first_sample_is_not_a_handshake(self):
        with self.assertRaisesRegex(ValueError, "disabled"):
            self.run_samples([missing(), row(43)])
        self.assertNotIn("ready", self.events)

    def test_partial_disabled_output_from_timeout_cannot_supply_handshake(self):
        with self.assertRaisesRegex(ValueError, "disabled"):
            self.run_samples([missing(row(22, "partial")), row()])
        self.assertNotIn("ready", self.events)

    def test_partial_code0_from_timeout_cannot_supply_enable_evidence(self):
        self.assertTrue(self.run_samples([row(22), missing(row(0, "partial")), row()]))
        recorded = [event[1] for event in self.events if isinstance(event, tuple)]
        self.assertEqual([item["sample"] for item in recorded], ["actual"])

    def test_only_timeouts_exhaust_attempts_without_forging_readiness(self):
        self.assertFalse(self.run_samples([missing(), missing()]))
        self.assertEqual(self.events, ["fetch", "fetch"])

    def test_missing_samples_after_handshake_never_invent_code0(self):
        self.assertFalse(self.run_samples([row(22), missing(), missing()]))
        self.assertEqual(self.events.count("ready"), 1)
        self.assertFalse(any(isinstance(event, tuple) for event in self.events))

    def check_timeout_owner_change(self, change):
        def restore():
            self.phase.write_text(json.dumps(self.owner))

        with self.assertRaisesRegex(ValueError, "identity"):
            self.run_samples([missing(), row(22), row()], change=change, pause=restore)
        self.assertNotIn("ready", self.events)

    def test_owner_change_during_timeout_is_refused_before_pause(self):
        self.check_timeout_owner_change({"pid": os.getpid() + 1})

    def test_reused_pid_during_timeout_is_refused_before_pause(self):
        self.check_timeout_owner_change({"starttime": 124})

    def test_disabled_result_after_total_deadline_never_creates_ready(self):
        def late():
            self.clock = 120
            return row(22)

        self.assertFalse(self.run_samples([late]))
        self.assertNotIn("ready", self.events)

    def test_code0_result_after_total_deadline_is_not_recorded(self):
        def late():
            self.clock = 120
            return row()

        self.assertFalse(self.run_samples([row(22), late]))
        self.assertFalse(any(isinstance(event, tuple) for event in self.events))

    def invoke_main(self, responses):
        ready = Path(self.temp.name) / "ready.json"
        diag = Path(self.temp.name) / "diag.txt"
        args = ["observer", str(self.phase), "on_precision", str(ready), str(diag),
                "--", "fixture-cim"]
        with mock.patch.object(sys, "argv", args), \
                mock.patch.object(O.subprocess, "run", side_effect=responses) as command, \
                mock.patch.object(O.time, "sleep"), \
                mock.patch.object(O.time, "monotonic", return_value=0), \
                mock.patch.object(G, "validate_phase_owner"), \
                mock.patch.object(G, "record_windows_init") as record, \
                contextlib.redirect_stdout(io.StringIO()):
            code = O.main()
        return code, ready, diag, command, record

    @staticmethod
    def response(code=0, text=None):
        return subprocess.CompletedProcess(["fixture-cim"], code, text or row(), "")

    def test_cli_outer_timeout_then_actual_handshake_and_code0(self):
        code, ready, diag, command, record = self.invoke_main([
            missing(), self.response(text=row(22)), self.response(),
        ])
        self.assertEqual(code, 0)
        self.assertTrue(ready.exists())
        self.assertEqual(diag.read_text(), row())
        self.assertEqual(record.call_count, 1)
        self.assertTrue(all(call.kwargs["timeout"] == 10 for call in command.call_args_list))

    def test_cli_inner_timeout_is_missing_initial_sample_not_state(self):
        code, ready, _, _, record = self.invoke_main([
            self.response(124, row(22, "partial")),
            self.response(text=row(22)), self.response(),
        ])
        self.assertEqual(code, 0)
        self.assertTrue(ready.exists())
        self.assertEqual(record.call_count, 1)

    def test_cli_inner_timeout_after_handshake_cannot_publish_its_output(self):
        code, _, diag, _, record = self.invoke_main([
            self.response(text=row(22)), self.response(124, row(0, "partial")),
            self.response(),
        ])
        self.assertEqual(code, 0)
        self.assertEqual(diag.read_text(), row())
        self.assertEqual(record.call_count, 1)

    def test_unknown_command_failure_before_handshake_remains_fatal(self):
        with self.assertRaisesRegex(ValueError, "diagnostic command failed: 13"):
            self.invoke_main([self.response(13)])
        self.assertFalse((Path(self.temp.name) / "ready.json").exists())

    def test_unknown_command_failure_after_handshake_remains_fatal(self):
        with self.assertRaisesRegex(ValueError, "diagnostic command failed: 13"):
            self.invoke_main([self.response(text=row(22)), self.response(13)])
        self.assertFalse((Path(self.temp.name) / "diag.txt").exists())


if __name__ == "__main__":
    unittest.main()
