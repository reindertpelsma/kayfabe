# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The P1+P2 box-log gate must report a fail on each defect it exists for, and pass a clean log.

A gate that only ever says PASS is the instrument failure this tree keeps naming: each case below
plants one defect into an otherwise clean synthetic log and asserts the gate catches it by name;
each positive-control mode is shown to fail on a log where its counter did NOT move.
"""

import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("tspace_log_gate", ROOT / "scripts/p1p2/tspace_log_gate.py")
gate_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate_mod)

STATUS = ("kf3: status mem[inval=3 walks=2/2 carve_gpu=0 carve_kernel=0 carve_cpu=0 fn70=1] "
          "rc[armed=1] inca[strict=yes counted=0 heap_out=0 rows_inexact=0] "
          "tspace[built=yes twin_refused=0 tspace_refused=0 slots_leaked=0 twin_freeing=0]")
CLEAN = [
    "kf3: P1+P2 T-space ON: built at prewarm (KF3_TSPACE; docs/design/V3_P1P2_TSPACE.md)",
    "kf3: mem t=0.412s tspace space=0x10 fb=0x120000000+0x2efbe0000 ram=0x410000000+0x200000000 "
    "(obj 0x20) rings=0xff00000000+0x100000000 build_us=306000",
    "kf3: mem t=0.500s [1/3] prewarm: spare host space 0x11 ready before the guest runs "
    "— windows=none (vaspace 10 us, ram_obj 1 us, total 12 us)",
    "kf3: VasKey(0xc1e00002_0000a001) mirror space=0x12: windows=none rings=none (40 us: vaspace 30 ram_obj 1)",
    "kf-host: channel birth h=0xcafe0020 engine=0xb reply_flags=0x00000080 PRIVILEGED_CHANNEL=0 privilege=USER cap_sys_admin=cleared-for-call",
    "kf3: chan 0xc1e00002:0xcaf BORN Translated: token 0x1 -> host 0x5 in VasKey(1)",
    "kf3: VasKey(0xc1d0002b_00010005) mirror space=0x13: windows=none rings=none (recycled)",
    "kf-host: channel birth h=0xcafe0024 engine=0x9 reply_flags=0x00000080 PRIVILEGED_CHANNEL=0 privilege=USER cap_sys_admin=cleared-for-call",
    "kf3: chan 0xc1d0002b:0xbee BORN Passthrough: token 0x2 -> host 0x6 privilege=0 rc=armed",
    "kf3: TSPACE-RETIRE tok=0x1 host=0x5 key=VasKey(1) ring_va=0xff00000000 released=true stale_binds=0/14 late=0 "
    "indeterminate=0",
    STATUS,
]


def failed(lines, **kw):
    return {name for name, ok, _ in gate_mod.gate(lines, **kw) if not ok}


def subst(old, new, lines=CLEAN):
    return [ln.replace(old, new) for ln in lines]


class TspaceLogGate(unittest.TestCase):
    def test_a_clean_tspace_log_passes(self):
        self.assertEqual(failed(CLEAN, user=True), set())

    def test_an_empty_log_is_a_failure_not_a_pass(self):
        self.assertEqual(failed([]), {"LOG"})

    def test_a_mirror_or_a_recycled_twin_with_windows_is_caught(self):
        bad = CLEAN + ["kf3: VasKey(7) mirror space=0x14: windows fb=0x1fffe00000000+0x300000000 ram=NONE rings=none (1 us)"]
        self.assertIn("WINDOWS=NONE", failed(bad))
        recycled = subst("windows=none rings=none (recycled)",
                         "windows fb=0x1fffe00000000+0x300000000 ram=NONE rings=none (recycled)")
        self.assertIn("WINDOWS=NONE", failed(recycled))

    def test_a_tspace_line_after_the_first_translated_birth_is_caught(self):
        bad = [CLEAN[0], CLEAN[5], CLEAN[1]] + CLEAN[2:5] + CLEAN[6:]
        self.assertIn("T-TSPACE-BUILD", failed(bad))

    def test_a_window_past_the_ring_region_is_caught(self):
        self.assertIn("T-TSPACE-BUILD", failed(subst("ram=0x410000000+0x200000000", "ram=0xff00000000+0x200000000")))

    def test_a_run_with_no_translated_birth_is_caught(self):
        self.assertIn("BORN-TRANSLATED", failed([ln for ln in CLEAN if "BORN Translated" not in ln]))

    def test_a_refusal_a_leak_a_carve_leaf_or_a_heap_leaf_is_caught(self):
        for field in ("twin_refused=0", "tspace_refused=0", "slots_leaked=0", "twin_freeing=0", "carve_gpu=0",
                      "heap_out=0"):
            self.assertIn("COUNTERS", failed(subst(field, field[:-1] + "3")), field)
        self.assertIn("COUNTERS", failed(subst(" carve_gpu=0 carve_kernel=0 carve_cpu=0", "")))

    def test_a_stale_bind_or_no_retire_is_caught(self):
        self.assertIn("STALE-BIND", failed(subst("stale_binds=0/14", "stale_binds=2/14")))
        self.assertIn("STALE-BIND", failed([ln for ln in CLEAN if "TSPACE-RETIRE" not in ln]))
        self.assertIn("STALE-BIND", failed(subst("stale_binds=0/14", "stale_binds=0/0")))

    def test_any_dead_translated_channel_is_caught(self):
        dead = CLEAN + ["kf3: chan token 0x1 (VasKey(1)) DEAD: ring: Read { gp: 0 }"]
        self.assertIn("NO-DEAD", failed(dead))
        other = CLEAN + ["kf3: chan token 0x9 (VasKey(1)) DEAD: ring: Read { gp: 0 }"]
        self.assertNotIn("NO-DEAD", failed(other))

    def test_p0_evidence_is_required_or_the_pass_is_scoped(self):
        no_p0 = [ln for ln in CLEAN if "kf-host: channel birth" not in ln]
        self.assertIn("P0-EVIDENCE", failed(no_p0))
        res = gate_mod.gate(no_p0, p0_pending=True)
        self.assertIn("P0-EVIDENCE(SCOPED)", {n for n, ok, _ in res if ok})
        self.assertIn("P0-EVIDENCE", failed(CLEAN + ["kf-host: refused: RM stamped PRIVILEGED_CHANNEL=1"]))
        self.assertIn("P0-EVIDENCE", failed([ln for ln in CLEAN if "h=0xcafe0024" not in ln]))

    def test_user_births_are_checked_only_when_asked(self):
        root_only = subst("privilege=0", "privilege=1")
        self.assertIn("USER-BIRTHS", failed(root_only, user=True))
        self.assertNotIn("USER-BIRTHS", failed(root_only))

    def test_the_known_positive_arm_must_see_windows(self):
        legacy = ["kf3: VasKey(7) mirror space=0x14: windows fb=0x1fffe00000000+0x300000000 ram=NONE rings=0x0+0x0 (1 us)"]
        self.assertEqual(failed(legacy, windows=True), set())
        self.assertEqual(failed(CLEAN, windows=True), {"WINDOWS-PRESENT"})

    def test_every_positive_control_must_move_its_counter(self):
        moved = {
            "stale": subst("stale_binds=0/14", "stale_binds=2/14"),
            "shadow": CLEAN + ["kf3: TSHADOW tok=0x1 host=0x5 key=VasKey(1) segments=3 items=3 launches=0 "
                               "max_pieces=0 would_refuse=[unclassified:3] resolve_miss=3 unknown_field=3 "
                               "unclassified=3 bound_after_split=0 unbound_at_free=0 NEGCTL"],
            "heap": subst("heap_out=0", "heap_out=4"),
            "carve": subst("carve_cpu=0", "carve_cpu=12"),
            "twin": subst("twin_refused=0", "twin_refused=2"),
            "oversize": subst("tspace_refused=0", "tspace_refused=5") + [
                "kf3: mem t=0.4s tspace: not built (guest-RAM window [0x410000000, 0xff00000000) reaches the ring region)"],
            "window": CLEAN + ["kf3: VasKey(7) mirror space=0x14: windows fb=0x1fffe00000000+0x300000000 ram=NONE "
                               "rings=none (1 us)"],
        }
        self.assertEqual(set(moved), set(gate_mod.NEGCTLS))
        for name, lines in moved.items():
            self.assertEqual(failed(lines, negctl=name), set(), name)
            self.assertNotEqual(failed(CLEAN, negctl=name), set(), f"{name} must fail when nothing moved")
        self.assertNotEqual(failed(CLEAN, negctl="nonsense"), set())
        # A TSHADOW line whose counters did not move is not the shadow control moving.
        still = CLEAN + ["kf3: TSHADOW tok=0x1 host=0x5 key=VasKey(1) segments=3 items=3 launches=0 "
                         "max_pieces=0 would_refuse=[] resolve_miss=0 unknown_field=0 unclassified=0"]
        self.assertEqual(failed(still, negctl="shadow"), {"SHADOW-NEGCTL"})

    def test_the_default_and_census_arms(self):
        default = [
            "kf3: P1+P2 T-space OFF: today's mirrors and windows",
            STATUS.replace("strict=yes", "strict=no").replace(
                " tspace[built=yes twin_refused=0 tspace_refused=0 slots_leaked=0 twin_freeing=0]", ""),
        ]
        self.assertEqual(failed(default, default=True), set())
        self.assertIn("INCA-COUNTERS", failed(subst("counted=0", "counted=7", default), default=True))
        self.assertIn("INCA-COUNTERS", failed(subst("rows_inexact=0", "rows_inexact=1", default), default=True))
        self.assertIn("DEFAULT-PATH", failed(default + [CLEAN[1]], default=True))
        census = default + [
            "kf3: TCENSUS tok=0x1 host=0x5 key=VasKey(0xc1e00002_0000a001) privilege=Some(..) methods=[...]",
            "kf3: TSHADOW tok=0x1 host=0x5 key=VasKey(1) segments=9 items=40 launches=6 max_pieces=2 "
            "would_refuse=[] resolve_miss=0 unknown_field=0 unclassified=0 bound_after_split=1 unbound_at_free=0",
        ]
        self.assertEqual(failed(census, census=True), set())
        self.assertIn("TCENSUS", failed(subst("VasKey(0xc1e00002", "VasKey(0xc1d0002b", census), census=True))
        for bad in ("max_pieces=4", "would_refuse=[unclassified:1]", "resolve_miss=2"):
            key = bad.split("=")[0]
            line = next(ln for ln in census if "TSHADOW" in ln)
            old = re_field(line, key)
            self.assertIn("TSHADOW", failed(subst(old, bad, census), census=True), bad)


def re_field(line, key):
    import re
    return re.search(key.replace("[", r"\[") + r"=(\[[^\]]*\]|\S+)", line).group(0)


if __name__ == "__main__":
    unittest.main()
