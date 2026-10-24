"""BW4 (D) REFERENCE lease: native windows-on-vast Windows 11 LTSC + 580.88 on the B1n die.

Tests first for the reference operator: offer scope and create arguments (template, no other
image), the Windows host-key pin from the console log, setup completion, and the diagnostic
PowerShell shapes (10-minute hold, disable/enable x3, BAR1, dumps, event log export).
"""

import ast
import importlib.util
import os
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
K = Path(os.environ.get("KF_BW3_K", HERE.parent / "attempt1" / "run_kit"))
WS = Path(os.environ.get("KF_BW3_D", "/workspace/bw4-ref-a1-run"))
KEY = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK0wmN/Cr3JXqmLW7u+g9pTh+wyqDHpSQEIQczXkVx9q"
OTHER = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsiGMUoGRh1jMRVYIWO2gKE"


def load(name):
    path = K / (name + ".py")
    if not path.exists():
        return None
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


R = load("ref_windows")


def log(*keys):
    lines = ["boot noise"]
    for key in keys:
        lines += [
            "Save this WINDOWS SSH HOST KEY before reboot:",
            key + " root@C.57500001",
            "256 x",
        ]
    return "\n".join(lines) + "\n"


class HostKeyPin(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(R, "ref_windows.py missing")

    def test_key_after_the_marker_is_pinned(self):
        self.assertEqual(R.windows_host_key(log(KEY)), KEY)

    def test_kernel_console_prefixed_lines_are_parsed(self):
        # MEASURED format, lease 57486060 console: "[   40.85] bash[1406]: <text>"
        text = (
            "[   40.853381] bash[1406]: Save this WINDOWS SSH HOST KEY before reboot:\n"
            f"[   40.855409] bash[2415]: {KEY} root@ubuntu\n"
            "[   40.861619] bash[2416]: 256 SHA256:x root@ubuntu (ED25519)\n"
        )
        self.assertEqual(R.windows_host_key(text), KEY)

    def test_the_same_key_printed_twice_is_one_pin(self):
        self.assertEqual(R.windows_host_key(log(KEY, KEY)), KEY)

    def test_two_different_keys_refuse(self):
        with self.assertRaises(ValueError):
            R.windows_host_key(log(KEY, OTHER))

    def test_absent_marker_is_none(self):
        self.assertIsNone(R.windows_host_key("boot\n" + KEY + "\n"))

    def test_non_ed25519_line_after_marker_refuses(self):
        with self.assertRaises(ValueError):
            R.windows_host_key("Save this WINDOWS SSH HOST KEY before reboot:\nssh-rsa AAAAB3 x\n")

    def test_known_hosts_line_pins_alias_and_key(self):
        line = R.known_hosts_line("bw-ref-57500001", KEY)
        self.assertEqual(line, "bw-ref-57500001 " + KEY + "\n")


class SetupState(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(R, "ref_windows.py missing")

    def test_complete_true_is_ready(self):
        self.assertTrue(R.setup_complete('﻿{"complete": true, "reboots": 1}\r\n'))

    def test_incomplete_or_garbage_is_not_ready(self):
        for text in ('{"complete": false}', "", "not json", '{"x": 1}', '{"complete": "true"}'):
            with self.subTest(text=text):
                self.assertFalse(R.setup_complete(text))


class ReferenceOperator(unittest.TestCase):
    """The wired reference ops.py (workspace) creates only from the named template."""

    def setUp(self):
        path = WS / "ops.py"
        if not path.exists():
            self.skipTest("reference workspace not wired yet")
        self.source = path.read_text()
        self.tree = ast.parse(self.source)

    def test_same_die_only(self):
        for node in ast.walk(self.tree):
            if isinstance(node, ast.Assign) and getattr(node.targets[0], "id", None) == "skus":
                self.assertEqual(ast.literal_eval(node.value), {"RTX_5070": "GB205"})
                return
        self.fail("skus table missing")

    def test_create_uses_the_template_and_no_other_image(self):
        self.assertIn(
            '"--template_hash",\n            "183b10af7fd788d08aca02e31811d70d",', self.source
        )
        self.assertNotIn('"--image"', self.source)
        self.assertIn('"--disk",\n            "150",', self.source)
        self.assertIn('"claude-bw4-REF1-NATIVE-20261024"', self.source)

    def test_guards_unchanged(self):
        self.assertIn('{"CU", "IR", "KP", "SY", "CN", "HK", "MO"}', self.source)
        self.assertIn('x.get("verification") in ("verified", "unverified")', self.source)
        self.assertEqual(self.source.count("stop_usd=2.0, cap_usd=2.5"), 3)
        self.assertIn("DL.require_receipt(", self.source)
        self.assertIn("precreate_watchdog.py", self.source)


class DiagnosticShapes(unittest.TestCase):
    def text(self, name):
        return (K / name).read_text()

    def assertIn(self, needle, haystack, msg=None):  # short failure text
        self.assertTrue(needle in haystack, msg or needle)

    def test_state_reads_bar1_driver_code_and_dumps(self):
        s = self.text("ref_state.ps1")
        for needle in (
            "BAR1",
            "nvidia-smi -q",
            "ConfigManagerErrorCode",
            "LiveKernelReports",
            "4101",
        ):
            self.assertIn(needle, s)

    def test_hold_is_ten_one_minute_samples(self):
        s = self.text("ref_hold.ps1")
        self.assertIn("1..10", s)
        self.assertIn("Start-Sleep -Seconds 60", s)
        self.assertIn("REF_HOLD_DONE", s)

    def test_cycle_is_three_bounded_disable_enable_rounds(self):
        s = self.text("ref_cycle.ps1")
        self.assertIn("1..3", s)
        self.assertEqual(s.count("pnputil /disable-device"), 1)
        self.assertEqual(s.count("pnputil /enable-device"), 1)
        self.assertIn("AddSeconds($seconds)", s)
        self.assertIn("REF_CYCLE_DONE", s)

    def test_capture_reuses_the_b1n_guest_capture(self):
        b1n = HERE.parents[1] / "ROUND_B1n_bw4/attempt1/run_kit/capture_guest.ps1"
        self.assertEqual(self.text("capture_guest.ps1"), b1n.read_text())


if __name__ == "__main__":
    unittest.main()
