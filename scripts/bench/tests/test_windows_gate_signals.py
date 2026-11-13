#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free regression tests for two log classifiers in scripts/bench/box/windows_gate.sh.

The gate's `--selftest` stubs both of them (`bar1_refusal() { return 0; }`, a hand-written `gsp_signals.txt`), so
before this file their real greps had no test. Both decide who is blamed:

  * `bar1_refusal` (d18032f3e): a kf3 refusal of the BAR1 size is the documented Vast exception, so it turns a
    guest failure into RETRY BAR1_EXCEPTION. It must read only the reason text after `realize refused`, because
    the device's own argument list names `bar1-size` too; a log that merely names the option must NOT hide a
    real kayfabe failure.
  * `refused_0x2080121f` in `gsp_signals.txt` (a5071c24b): the no-flags signature is a REFUSAL of
    GFX_POOL_QUERY_SIZE. The control's served trace line for the same control (`result=0x0`) is not one, and
    must not turn a PASS into FAIL KAYFABE.

Each function is cut out of the script and run against fixture logs; nothing needs a box.

    KF_WINDOWS_GATE=<path>   aim these tests at a mutated copy
"""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GATE = Path(os.environ.get("KF_WINDOWS_GATE") or ROOT / "scripts/bench/box/windows_gate.sh")


def bash(script):
    return subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)


def extract(name):
    r = subprocess.run(
        ["sed", "-n", f"/^{name}() {{/,/^}}/p", str(GATE)], capture_output=True, text=True, check=True
    )
    assert r.stdout.strip().endswith("}"), f"{name} moved or was renamed in windows_gate.sh"
    return r.stdout


class Fixture(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.TemporaryDirectory()
        self.addCleanup(self.d.cleanup)
        self.vm = Path(self.d.name, "vm")
        (self.vm / "logs").mkdir(parents=True)
        self.ev = Path(self.d.name, "ev")
        self.ev.mkdir()

    def qemu(self, text, boot=1):
        (self.vm / "logs" / f"T_b{boot}_qemu.log").write_text(text)


class Bar1Refusal(Fixture):
    def run_fn(self, ssh=False):
        if ssh:
            (self.ev / "ssh_seen").write_text("SSH_UP\n")
        script = f"VM={self.vm}; TAG=T; EV={self.ev}\n{extract('ssh_seen')}\n{extract('bar1_refusal')}\nbar1_refusal && echo YES || echo NO\n"
        return bash(script).stdout.strip()

    def test_a_refused_bar1_size_is_the_exception(self):
        self.qemu("kf3: realize refused: bar1-size 0x100000000 is not available on this host (BAR1 limit)\n")
        self.assertEqual(self.run_fn(), "YES")

    def test_the_devices_own_argument_list_naming_bar1_is_not_a_refusal(self):
        self.qemu("kf3 args: bar1-size=0x100000000 gpu-uuid=x; realize refused: the RM client was denied\n")
        self.assertEqual(self.run_fn(), "NO")

    def test_an_argument_list_with_a_power_of_two_note_is_not_a_refusal_either(self):
        self.qemu("kf3 realize: bar1-size=4G must be a power of two (ok); realize refused: no GPU\n")
        self.assertEqual(self.run_fn(), "NO")

    def test_a_log_with_no_refusal_at_all_is_not_one(self):
        self.qemu("-device kf3,bar1-size=0x100000000\nGPU booted\n")
        self.assertEqual(self.run_fn(), "NO")

    def test_a_guest_that_reached_ssh_was_not_stopped_by_bar1(self):
        self.qemu("kf3: realize refused: bar1-size too large\n")
        self.assertEqual(self.run_fn(ssh=True), "NO")

    def test_no_boot_1_log_is_not_a_refusal(self):
        self.assertEqual(self.run_fn(), "NO")


class NoFlagsSignature(Fixture):
    def signals(self, text):
        self.qemu(text, boot=2)
        script = (
            f"VM={self.vm}; TAG=T; EV={self.ev}\n{extract('collect_evidence')}\ncollect_evidence\n"
        )
        r = bash(script)
        self.assertEqual(r.returncode, 0, r.stderr)
        sig = (self.ev / "gsp_signals.txt").read_text()
        return {k: v for k, _, v in (l.partition("=") for l in sig.splitlines() if "=" in l)}

    def test_a_refusal_line_counts_as_the_no_flags_signature(self):
        s = self.signals("kf3: GSP REFUSED 0x2080121f GFX_POOL_QUERY_SIZE (no host-ABI row)\n")
        self.assertEqual(s["refused_0x2080121f"], "1")
        self.assertEqual(s["served_0x2080121f"], "0")

    def test_the_controls_served_trace_line_is_not_a_refusal(self):
        s = self.signals("rpc-trace cmd=0x2080121f size=8 result=0x0\n")
        self.assertEqual(s["refused_0x2080121f"], "0")
        self.assertEqual(s["served_0x2080121f"], "1")

    def test_a_refusal_of_another_control_does_not_count(self):
        s = self.signals("kf3: GSP REFUSED 0x20801234 something\nrpc-trace cmd=0x2080121f result=0x1\n")
        self.assertEqual(s["refused_0x2080121f"], "0")
        self.assertEqual(s["served_0x2080121f"], "0")

    def test_the_last_boots_log_is_the_one_read(self):
        self.qemu("kf3: GSP REFUSED 0x2080121f first boot\n", boot=1)
        (self.vm / "logs" / "T_b1_qemu.log").touch()
        s = self.signals("rpc-trace cmd=0x2080121f result=0x0\n")  # boot 2, written later and newest
        self.assertEqual(s["refused_0x2080121f"], "0")


if __name__ == "__main__":
    unittest.main()
