# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The P1+P2 box-log gate must report a fail on each defect it exists for, and pass a clean log.

A gate that only ever says PASS is the instrument failure this tree keeps naming: each case below
plants one defect into an otherwise clean synthetic log and asserts the gate catches it by name.
"""

import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("tspace_log_gate", ROOT / "scripts/p1p2/tspace_log_gate.py")
gate_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate_mod)

CLEAN = [
    "kf3: P1+P2 T-space ON: built at prewarm (KF3_TSPACE; docs/design/V3_P1P2_TSPACE.md)",
    "kf3: mem t=0.412s tspace space=0x10 fb=0x120000000+0x2efbe0000 ram=0x410000000+0x200000000 "
    "(obj 0x20) rings=0xff00000000+0x100000000 build_us=306000",
    "kf3: mem t=0.500s [1/3] prewarm: spare host space 0x11 ready before the guest runs "
    "— windows=none (vaspace 10 us, ram_obj 1 us, total 12 us)",
    "kf3: VasKey(0xc1e00002_0000a001) mirror space=0x12: windows=none rings=none (40 us: vaspace 30 ram_obj 1)",
    "kf3: chan 0xc1e00002:0xcaf BORN Translated: token 0x1 -> host 0x5",
    "kf3: VasKey(0xc1d0002b_00010005) mirror space=0x13: windows=none rings=none (40 us: vaspace 30 ram_obj 1)",
    "kf3: chan 0xc1d0002b:0xbee BORN Passthrough: token 0x2 -> host 0x6 privilege=0 rc=armed",
    "kf3: TSPACE-RETIRE tok=0x1 host=0x5 key=VasKey(1) ring_va=0xff00000000 released=true stale_binds=0/14",
    "kf3: status ... heap_refused=0 tspace[built=yes twin_refused=0 tspace_refused=0 slots_leaked=0]",
]


def failed(lines, **kw):
    return {name for name, ok, _ in gate_mod.gate(lines, **kw) if not ok}


class TspaceLogGate(unittest.TestCase):
    def test_a_clean_tspace_log_passes(self):
        self.assertEqual(failed(CLEAN), set())

    def test_an_empty_log_is_a_failure_not_a_pass(self):
        self.assertEqual(failed([]), {"LOG"})

    def test_a_mirror_with_windows_is_caught(self):
        bad = CLEAN + ["kf3: VasKey(7) mirror space=0x14: windows fb=0x1fffe00000000+0x300000000 ram=NONE rings=0x0+0x0 (1 us)"]
        self.assertIn("WINDOWS=NONE", failed(bad))

    def test_a_tspace_line_after_the_first_translated_birth_is_caught(self):
        bad = [CLEAN[0], CLEAN[4], CLEAN[1]] + CLEAN[2:4] + CLEAN[5:]
        self.assertIn("T-TSPACE-BUILD", failed(bad))

    def test_a_window_past_the_ring_region_is_caught(self):
        bad = [ln.replace("ram=0x410000000+0x200000000", "ram=0xff00000000+0x200000000") for ln in CLEAN]
        self.assertIn("T-TSPACE-BUILD", failed(bad))

    def test_a_refusal_or_a_leak_is_caught(self):
        for field in ("twin_refused=0", "tspace_refused=0", "slots_leaked=0"):
            bad = [ln.replace(field, field[:-1] + "3") for ln in CLEAN]
            self.assertIn("COUNTERS", failed(bad), field)

    def test_a_stale_bind_is_caught_and_the_positive_control_must_move(self):
        bad = [ln.replace("stale_binds=0/14", "stale_binds=2/14") for ln in CLEAN]
        self.assertIn("STALE-BIND", failed(bad))
        self.assertIn("STALE-BIND-NEGCTL", failed(CLEAN, negctl_stale=True))
        self.assertNotIn("STALE-BIND-NEGCTL", failed(bad, negctl_stale=True))

    def test_a_dead_bind_and_a_root_only_run_are_caught(self):
        self.assertIn("NO-DEAD-BIND", failed(CLEAN + ["kf3: chan token 0x1 (VasKey(1)) DEAD: tspace bind: unclassified"]))
        root_only = [ln.replace("privilege=0", "privilege=1") for ln in CLEAN]
        self.assertIn("USER-BIRTHS", failed(root_only))

    def test_the_known_positive_arm_must_see_windows(self):
        legacy = ["kf3: VasKey(7) mirror space=0x14: windows fb=0x1fffe00000000+0x300000000 ram=NONE rings=0x0+0x0 (1 us)"]
        self.assertEqual(failed(legacy, windows=True), set())
        self.assertEqual(failed(CLEAN, windows=True), {"WINDOWS-PRESENT"})


if __name__ == "__main__":
    unittest.main()
