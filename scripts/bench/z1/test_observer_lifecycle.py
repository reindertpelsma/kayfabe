"""Observe a normal Popen exit without confusing reaping with PID reuse."""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import observe_rootless as observer


class ObserverStartup(unittest.TestCase):
    def fixture(self, root):
        manifest = root / "manifest.json"
        receipt = root / "receipt.json"
        manifest.write_text(json.dumps({"revision": "a" * 40, "qemu": {"path": "/bin/true"}}))
        receipt.write_text(json.dumps({"approved_native_xid_lines": []}))
        return [
            "observe",
            str(manifest),
            str(receipt),
            str(root / "processes"),
            str(root / "ack"),
            "/bin/true",
        ]

    def test_reads_kernel_scope_before_starting_suite(self):
        with tempfile.TemporaryDirectory() as folder:
            with patch.object(sys, "argv", self.fixture(Path(folder))):
                with patch.object(observer.guard, "pin_suite", return_value={}):
                    with patch.object(observer, "snapshot", return_value="scope") as read:
                        with patch.object(observer.guard, "check_unchanged"):
                            with patch.object(
                                observer.subprocess,
                                "Popen",
                                side_effect=RuntimeError("launch reached"),
                            ):
                                with self.assertRaisesRegex(RuntimeError, "launch reached"):
                                    observer.main()
                        read.assert_called_once()

    def test_bad_initial_scope_stops_before_launch(self):
        with tempfile.TemporaryDirectory() as folder:
            with patch.object(sys, "argv", self.fixture(Path(folder))):
                with patch.object(observer.guard, "pin_suite", return_value={}):
                    with patch.object(observer, "snapshot", side_effect=ValueError("bad scope")):
                        with patch.object(observer.subprocess, "Popen") as launch:
                            with self.assertRaisesRegex(ValueError, "bad scope"):
                                observer.main()
                            launch.assert_not_called()


class ObserverExit(unittest.TestCase):
    def setUp(self):
        self.context = {"pid": 123, "starttime": 456}

    def test_reaped_child_without_publication_is_pending(self):
        with patch.object(observer.guard, "process_identity", return_value=None):
            self.assertIsNone(observer.child_phase(self.context, {}))

    def test_reaped_child_is_never_accepted_as_complete_without_exit_record(self):
        with patch.object(observer.guard, "process_identity", return_value=None):
            self.assertNotEqual(observer.child_phase(self.context, {}), "complete")

    def test_published_exit_is_complete(self):
        self.assertEqual(observer.child_phase(self.context, {"rc": 0}), "complete")

    def test_reused_pid_still_refuses(self):
        with patch.object(observer.guard, "process_identity", return_value=(999, 457)):
            with self.assertRaises(observer.guard.GuardRefused):
                observer.child_phase(self.context, {})

    def test_second_reused_pid_still_refuses(self):
        with patch.object(observer.guard, "process_identity", return_value=(999, 999)):
            with self.assertRaises(observer.guard.GuardRefused):
                observer.child_phase(self.context, {})


if __name__ == "__main__":
    unittest.main()
