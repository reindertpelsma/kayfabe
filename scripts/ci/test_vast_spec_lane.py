#!/usr/bin/env python3
"""The decisions of the SPECviewperf gate stage (scripts/bench/specviewperf/lane.py, tarfilter.py), GPU-free.

FALSIFIER (written before the code): a viewset with a failed subtest, a missing subtest, a non-zero exit, a guest
reset, a refusal by SPEC's validation or a timeout is graded PASS; a viewset cut short by the clock is graded FAIL
(or a hang past its own budget is graded RETRY); a stage verdict is PASS with one viewset not passed, with a red
host reference or with a new host Xid; a RETRY class hides a failure; a log line that is not a status line survives
the scrub; the tar filter keeps another lane's viewset, drops anything else, or accepts a link or a `..`.

Replayed real evidence: the 2026-11-05 kayfabe run (traces/specviewperf_vast_20261105/spec/: medical-04 passed,
catia-07 failed 2 of 6 subtests with 20 resets) and the 2026-11-07 native reference run of catia-07
(scripts/bench/specviewperf/fixtures/). The END lines of the kayfabe run were not kept; the tests construct them.
"""

import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
SPEC = HERE.parent / "bench/specviewperf"
sys.path.insert(0, str(SPEC))
import lane  # noqa: E402
import tarfilter  # noqa: E402

TRACE = HERE.parents[1] / "traces/specviewperf_vast_20261105/spec"
FIX = SPEC / "fixtures"
TBL = lane.table()


def text(p):
    return Path(p).read_text()


END_OK = "SPECRUN_END workload=%s rc=0 start=a end=b secs=60\n"
END_BAD = "SPECRUN_END workload=%s rc=1 start=a end=b secs=60\n"
TDR0 = "GATE_TDR since=a display4101=0 nvlddmkm153=0 kernelpower41=0 eventlog6008=0 bugcheck1001=0 livekernelevent_other=0 livekernelevent193=0 livekernelevent_stale=0 strict=0 total=0\n"
TDR7 = TDR0.replace("nvlddmkm153=0", "nvlddmkm153=7").replace("total=0", "total=7")


def cli(w, statuses, workload="Completed", listed=0):
    out = ["[Workload:%s] Starting" % w]
    out += ["    ✓ <id=%d> %s " % (i, n) for i, (n, _) in enumerate(statuses[:listed])]
    for n, st in statuses:
        out.append("[workload: %s / subtest: %s] Started" % (w, n))
        out.append("[workload: %s / subtest: %s] %s" % (w, n, st))
    if workload:
        out.append("[Workload: %s] %s" % (w, workload))
    return "\n".join(out) + "\n"


OK3 = [("01_A", "Completed"), ("02_B", "Completed"), ("03_C", "Completed")]


def grade(w="catia-07", c=None, end=None, tdr=TDR0, timeout=None):
    c = cli(w, OK3) if c is None else c
    end = END_OK % w if end is None else end
    return lane.grade_viewset(w, c, end, tdr, timeout)


class Lanes(unittest.TestCase):
    def test_the_two_lanes_are_disjoint_and_cover_the_nine_viewsets(self):
        a, b = lane.lane_ids("lanea", TBL), lane.lane_ids("laneb", TBL)
        self.assertFalse(set(a) & set(b))
        self.assertEqual(set(a) | set(b), set(TBL["viewsets"]))
        self.assertEqual(len(TBL["viewsets"]), 9)

    def test_lane_a_is_enscape_blender_creo_energy_and_lane_b_the_rest(self):
        self.assertEqual(set(lane.lane_ids("lanea", TBL)), {"enscape-01", "blender-01", "creo-04", "energy-04"})
        self.assertEqual(set(lane.lane_ids("laneb", TBL)), {"catia-07", "unreal_engine-01", "maya-07", "solidworks-08", "medical-04"})

    def test_both_is_lane_a_then_lane_b(self):
        self.assertEqual(lane.lane_ids("both", TBL), lane.lane_ids("lanea", TBL) + lane.lane_ids("laneb", TBL))

    def test_an_unknown_lane_is_refused(self):
        for bad in ("", "lanec", "LANEA", "lanea,laneb"):
            with self.assertRaises(ValueError):
                lane.lane_ids(bad, TBL)

    def test_a_lane_runs_its_cheapest_viewset_first(self):
        for name in ("lanea", "laneb"):
            walls = [TBL["viewsets"][n]["native_s"] for n in lane.lane_ids(name, TBL)]
            self.assertEqual(walls, sorted(walls))

    def test_every_viewset_name_is_a_workload_name(self):
        for n in TBL["viewsets"]:
            self.assertRegex(n, lane.WORKLOAD.pattern)

    def test_the_native_walls_are_the_measured_ones(self):
        # traces/specviewperf_native_20261107/README.md, results table (4090, boxes A and B)
        measured = {"blender-01": 1876, "catia-07": 803, "creo-04": 1243, "energy-04": 323, "enscape-01": 2042,
                    "maya-07": 400, "medical-04": 318, "solidworks-08": 989, "unreal_engine-01": 1236}
        self.assertEqual({k: v["native_s"] for k, v in TBL["viewsets"].items()}, measured)

    def test_every_budget_is_above_the_native_wall_with_room(self):
        for n, v in TBL["viewsets"].items():
            self.assertGreaterEqual(v["budget_s"], 2 * v["native_s"], n)
            self.assertGreaterEqual(v["budget_s"], 1200, n)

    def test_the_whole_lane_time_is_setup_plus_margin_plus_tail(self):
        native = sum(TBL["viewsets"][n]["native_s"] for n in lane.lane_ids("lanea", TBL))
        self.assertEqual(native, 5484)
        self.assertEqual(lane.whole_lane_s("lanea", TBL), int(2700 + 1.25 * 5484 + 600))
        self.assertGreater(lane.whole_lane_s("both", TBL), lane.whole_lane_s("lanea", TBL))
        self.assertGreater(lane.whole_lane_s("both", TBL), lane.whole_lane_s("laneb", TBL))

    def test_the_settle_time_is_at_least_five_minutes(self):
        self.assertGreaterEqual(TBL["settle_s"], 300)

    def test_the_disk_need_covers_the_archive_and_the_trees(self):
        for name in ("lanea", "laneb", "both"):
            tree = sum(TBL["viewsets"][n]["tree_gb"] for n in lane.lane_ids(name, TBL))
            self.assertGreater(TBL["min_free_gb"][name], 42 + tree, name)

    def test_the_payload_reads_the_same_table_and_the_same_numbers(self):
        import importlib.util

        s = importlib.util.spec_from_file_location("payload_for_lane", HERE / "vast_payload.py")
        m = importlib.util.module_from_spec(s)
        s.loader.exec_module(m)
        table = m.spec_table(HERE.parents[1])
        for name in ("lanea", "laneb", "both"):
            self.assertEqual(m.spec_names(table, name), lane.lane_ids(name, TBL))
            self.assertEqual(m.spec_min_remaining_ms(table, name), lane.whole_lane_s(name, TBL) * 1000)


class TimeoutFor(unittest.TestCase):
    NEED = int(1.25 * 323)  # energy-04

    def test_too_little_time_skips_the_viewset(self):
        self.assertEqual(lane.timeout_for("energy-04", 600 + self.NEED - 1, TBL), ("skip", 0))
        self.assertEqual(lane.timeout_for("energy-04", 0, TBL), ("skip", 0))
        self.assertEqual(lane.timeout_for("energy-04", 100, TBL), ("skip", 0))

    def test_exactly_the_need_plus_the_tail_starts_it_with_a_cut_budget(self):
        self.assertEqual(lane.timeout_for("energy-04", 600 + self.NEED, TBL), ("cut", self.NEED))

    def test_between_the_need_and_the_budget_the_time_left_is_the_cut_budget(self):
        self.assertEqual(lane.timeout_for("energy-04", 600 + 1000, TBL), ("cut", 1000))
        self.assertEqual(lane.timeout_for("creo-04", 600 + 2000, TBL), ("cut", 2000))

    def test_with_enough_time_the_viewsets_own_budget_applies(self):
        self.assertEqual(lane.timeout_for("energy-04", 600 + 1200, TBL), ("full", 1200))
        self.assertEqual(lane.timeout_for("blender-01", 100000, TBL), ("full", 5700))

    def test_the_tail_reserve_is_never_spent(self):
        mode, secs = lane.timeout_for("creo-04", 3000, TBL)
        self.assertLessEqual(secs, 3000 - 600)

    def test_each_viewset_has_its_own_need(self):
        self.assertGreater(lane.need_s("enscape-01", TBL), lane.need_s("energy-04", TBL))
        self.assertEqual(lane.timeout_for("enscape-01", 600 + lane.need_s("energy-04", TBL) + 10, TBL), ("skip", 0))


class GradeViewset(unittest.TestCase):
    # --- PASS needs every condition -------------------------------------------------------------
    def test_a_completed_viewset_with_every_subtest_completed_exit_0_and_no_reset_passes(self):
        r = grade()
        self.assertEqual((r["result"], r["reason"], r["done"], r["total"]), ("PASS", "-", 3, 3))

    def test_the_real_native_catia_run_passes(self):
        r = lane.grade_viewset("catia-07", text(FIX / "native_catia-07.cli.log"), text(FIX / "native_catia-07.END.txt"),
                               text(FIX / "native_catia-07.tdr.txt"))
        self.assertEqual((r["result"], r["done"], r["total"]), ("PASS", 6, 6))

    def test_the_real_kayfabe_medical_run_passes(self):
        r = lane.grade_viewset("medical-04", text(TRACE / "status_medical-04.txt"), END_OK % "medical-04", text(TRACE / "tdr_after_med.txt"))
        self.assertEqual((r["result"], r["done"], r["total"]), ("PASS", 10, 10))

    def test_the_real_kayfabe_solidworks_maya_and_energy_runs_pass(self):
        for w, tdr in (("solidworks-08", "tdr_after_solidworks-08.txt"), ("maya-07", "tdr_after_maya-07.txt"), ("energy-04", "tdr_after_energy-04.txt")):
            r = lane.grade_viewset(w, text(TRACE / ("status_%s.txt" % w)), END_OK % w, text(TRACE / tdr))
            self.assertEqual(r["result"], "PASS", w)

    # --- the same viewset failing, as kayfabe did on 2026-11-05 -----------------------------------
    def test_the_real_kayfabe_catia_failure_is_a_fail_with_the_failed_subtests_named(self):
        r = lane.grade_viewset("catia-07", text(TRACE / "status_catia-07.txt"), END_BAD % "catia-07", text(TRACE / "tdr_after_catia-07.txt"))
        self.assertEqual(r["result"], "FAIL")
        self.assertEqual(r["failed"], ["03_ThreeLoftJetsDarkMirror", "06_LoftJets"])
        self.assertEqual((r["done"], r["total"]), (4, 6))

    def test_the_real_catia_failure_is_a_fail_even_without_the_reset_counters(self):
        r = lane.grade_viewset("catia-07", text(TRACE / "status_catia-07.txt"), END_BAD % "catia-07", TDR0)
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "SUBTEST"))

    def test_the_real_catia_failure_is_a_fail_with_a_zero_exit_too(self):
        r = lane.grade_viewset("catia-07", text(TRACE / "status_catia-07.txt"), END_OK % "catia-07", TDR0)
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "SUBTEST"))

    # --- one test per way a PASS could be forged -------------------------------------------------
    def test_a_failed_subtest_is_a_fail(self):
        c = cli("catia-07", [("01_A", "Completed"), ("02_B", "Failed"), ("03_C", "Completed")], "Failed")
        self.assertEqual(grade(c=c)["result"], "FAIL")

    def test_another_failed_subtest_is_a_fail_with_its_name(self):
        c = cli("maya-07", [("01_A", "Failed"), ("02_B", "Completed")], "Completed")
        r = grade("maya-07", c)
        self.assertEqual((r["result"], r["failed"]), ("FAIL", ["01_A"]))

    def test_a_subtest_that_started_and_never_finished_is_a_fail(self):
        c = cli("catia-07", [("01_A", "Completed"), ("02_B", "Started")], "")
        self.assertEqual(grade(c=c)["result"], "FAIL")

    def test_a_viewset_whose_listing_names_more_subtests_than_ran_is_a_fail(self):
        c = cli("catia-07", OK3, "Completed", listed=3) + "    ✓ <id=9> 04_NeverRan \n"
        r = grade(c=c)
        self.assertEqual((r["result"], r["total"], r["done"]), ("FAIL", 4, 3))

    def test_a_listing_with_one_more_subtest_than_completed_is_a_fail_with_the_counts(self):
        c = cli("maya-07", OK3[:2], "Completed", listed=2) + "    ✓ <id=7> 03_Missing \n"
        r = grade("maya-07", c)
        self.assertEqual((r["result"], r["done"], r["total"]), ("FAIL", 2, 3))

    def test_a_viewset_that_never_reached_completed_is_a_fail(self):
        self.assertEqual(grade(c=cli("catia-07", OK3, "Failed"))["result"], "FAIL")

    def test_a_viewset_without_a_workload_status_is_a_fail(self):
        self.assertEqual(grade(c=cli("catia-07", OK3, ""))["result"], "FAIL")

    def test_a_viewset_with_no_subtest_at_all_is_not_a_pass(self):
        r = grade(c="[Workload:catia-07] Starting\n[Workload: catia-07] Completed\n")
        self.assertNotEqual(r["result"], "PASS")

    def test_a_completed_viewset_that_ran_no_subtest_is_not_a_pass(self):
        r = grade(c="[Workload: catia-07] Completed\n")
        self.assertNotEqual(r["result"], "PASS")

    def test_a_non_zero_exit_under_a_completed_viewset_is_a_fail(self):
        r = grade(end=END_BAD % "catia-07")
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "RC"))

    def test_another_non_zero_exit_is_a_fail(self):
        r = grade(end="SPECRUN_END workload=catia-07 rc=-1 start=a end=b secs=1\n")
        self.assertEqual(r["result"], "FAIL")

    def test_a_guest_reset_under_a_completed_viewset_is_a_fail(self):
        r = grade(tdr=TDR7)
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "RESET"))

    def test_a_single_reset_is_enough(self):
        one = TDR0.replace("display4101=0", "display4101=1").replace("total=0", "total=1")
        self.assertEqual(grade(tdr=one)["reason"], "RESET")

    def test_the_known_live_dump_193_alone_does_not_fail_a_viewset(self):
        # as windows_gate.sh: code 193 is counted apart and gates only with -Strict (total excludes it)
        t = TDR0.replace("livekernelevent193=0", "livekernelevent193=2")
        self.assertEqual(grade(tdr=t)["result"], "PASS")

    def test_a_wrong_workload_in_the_log_is_not_this_viewsets_pass(self):
        r = grade(c=cli("maya-07", OK3))
        self.assertNotEqual(r["result"], "PASS")

    # --- refusals by SPEC's own validation are failures, never exclusions ----------------------------
    def test_a_viewset_refused_by_validation_is_a_fail(self):
        c = "Error: Requirement: raytracing\n  Requirement: sufficientVideoMemory\n[Workload: enscape-01] Failed -- check logs for errors\n"
        r = grade("enscape-01", c, END_BAD % "enscape-01")
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "REFUSED"))

    def test_a_viewset_that_does_not_meet_the_memory_requirement_is_a_fail(self):
        c = "[Workload: energy-04] Failed\nthe system does not meet the CPU memory requirements. Found: 8.00 GB\n"
        r = grade("energy-04", c, END_BAD % "energy-04")
        self.assertEqual(r["result"], "FAIL")

    def test_no_compatible_display_is_a_fail_under_kayfabe(self):
        c = "Error: System does not support 4K or Full HD display resolution. No compatible resolution found.\n"
        r = grade("medical-04", c, END_BAD % "medical-04")
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "DISPLAY"))

    # --- timeouts -------------------------------------------------------------------------------
    def test_a_viewset_past_its_own_budget_is_a_fail_whatever_it_printed(self):
        r = grade(c=cli("catia-07", [("01_A", "Completed")], ""), end="", tdr="", timeout="full")
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "TIMEOUT"))

    def test_a_full_timeout_is_a_fail_even_if_everything_printed_looks_complete(self):
        self.assertEqual(grade(timeout="full")["result"], "FAIL")

    def test_a_viewset_cut_by_the_clock_is_not_run_class_time_never_a_fail(self):
        r = grade(c=cli("catia-07", [("01_A", "Completed")], ""), end="", tdr="", timeout="cut")
        self.assertEqual((r["result"], r["reason"]), ("NOT_RUN", "TIME"))

    def test_a_cut_viewset_is_never_a_pass_even_if_it_printed_everything(self):
        self.assertEqual(grade(timeout="cut")["result"], "NOT_RUN")

    def test_a_guest_that_died_during_the_run_is_the_guests_failure(self):
        r = grade(c="", end="", tdr="", timeout="dead")
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "GUEST"))

    def test_a_launch_the_guest_script_refused_is_the_harness_not_the_guest(self):
        r = grade(c="", end="", tdr="", timeout="launch")
        self.assertEqual((r["result"], r["reason"]), ("NOT_RUN", "LAUNCH"))

    def test_a_refused_launch_is_never_a_pass_or_a_fail(self):
        for w in ("catia-07", "maya-07"):
            r = grade(w, cli(w, OK3), END_OK % w, TDR0, timeout="launch")
            self.assertEqual(r["result"], "NOT_RUN")

    def test_a_guest_that_went_silent_during_the_run_is_the_guests_failure(self):
        r = grade(c="", end="", tdr="", timeout="silent")
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "GUEST"))

    # --- harness traps are not verdicts ------------------------------------------------------------------
    def test_the_gather_system_details_trap_is_not_run_not_a_fail(self):
        c = "Error: Unable to gather system details: ... gwpg_sysinfo.exe -s -d\n"
        r = grade(c=c, end=END_BAD % "catia-07")
        self.assertEqual((r["result"], r["reason"]), ("NOT_RUN", "GATHER"))

    def test_the_gather_trap_after_subtests_started_is_not_the_trap(self):
        c = cli("catia-07", [("01_A", "Failed")], "Failed") + "Error: Unable to gather system details\n"
        self.assertEqual(grade(c=c, end=END_BAD % "catia-07")["result"], "FAIL")

    def test_missing_end_and_reset_counters_are_not_run_not_a_pass(self):
        r = grade(end="", tdr="")
        self.assertEqual((r["result"], r["reason"]), ("NOT_RUN", "NO_TDR"))

    def test_missing_reset_counters_alone_are_not_run_never_a_pass(self):
        r = grade(tdr="")
        self.assertEqual((r["result"], r["reason"]), ("NOT_RUN", "NO_TDR"))

    def test_the_row_line_matches_the_strict_status_pattern(self):
        for r in (grade(), grade(tdr=TDR7), grade(timeout="cut"), grade("unreal_engine-01", cli("unreal_engine-01", OK3), END_OK % "unreal_engine-01")):
            line = lane.row_line(r)
            self.assertIsNotNone(lane.ROW_PATTERN.match(line), line)
            self.assertEqual(lane.parse_row(line)["name"], r["name"])

    def test_the_row_line_carries_no_free_text(self):
        c = cli("catia-07", [("01_A_secret_scene_name_with_a_score_9999", "Failed")], "Failed")
        line = lane.row_line(grade(c=c, end=END_BAD % "catia-07"))
        self.assertIsNotNone(lane.ROW_PATTERN.match(line), line)


def rows(**kw):
    out = []
    for n in lane.lane_ids(kw.pop("lane", "lanea"), TBL):
        res, reason = kw.get(n, ("PASS", "-"))
        out.append({"name": n, "result": res, "reason": reason, "done": 3, "total": 3, "failed": []})
    return out


class Decide(unittest.TestCase):
    def d(self, rs=None, lane_="lanea", before="green", after="green", bar1=False, xid=0, abort=None):
        return lane.decide(rows(lane=lane_) if rs is None else rs, lane_, before, after, bar1, xid, abort, TBL)

    def test_every_viewset_passed_with_green_references_and_no_xid_is_a_pass(self):
        self.assertEqual(self.d(), ("PASS", "NONE"))
        self.assertEqual(self.d(rows(lane="laneb"), "laneb"), ("PASS", "NONE"))

    def test_one_failed_viewset_is_a_fail_attributed_to_kayfabe(self):
        self.assertEqual(self.d(rows(**{"creo-04": ("FAIL", "SUBTEST")})), ("FAIL", "KAYFABE"))
        self.assertEqual(self.d(rows(lane="laneb", **{"catia-07": ("FAIL", "RESET")}), "laneb"), ("FAIL", "KAYFABE"))

    def test_a_fail_with_a_red_reference_before_is_a_retry_hostref(self):
        self.assertEqual(self.d(rows(**{"creo-04": ("FAIL", "SUBTEST")}), before="RED"), ("RETRY", "HOSTREF"))

    def test_a_fail_with_a_red_reference_after_is_a_retry_hostref(self):
        self.assertEqual(self.d(rows(**{"creo-04": ("FAIL", "SUBTEST")}), after="RED"), ("RETRY", "HOSTREF"))

    def test_a_fail_after_an_unrun_reference_is_a_retry_hostref(self):
        self.assertEqual(self.d(rows(**{"creo-04": ("FAIL", "SUBTEST")}), after="unrun"), ("RETRY", "HOSTREF"))

    def test_the_bar1_exception_makes_a_fail_a_retry(self):
        self.assertEqual(self.d(rows(**{"energy-04": ("FAIL", "GUEST")}), bar1=True), ("RETRY", "BAR1_EXCEPTION"))

    def test_a_viewset_not_run_is_a_retry_never_a_pass(self):
        self.assertEqual(self.d(rows(**{"enscape-01": ("NOT_RUN", "TIME")})), ("RETRY", "TIME"))

    def test_a_missing_row_is_a_retry_never_a_pass(self):
        self.assertEqual(self.d(rows()[:-1])[0], "RETRY")
        self.assertEqual(self.d([])[0], "RETRY")

    def test_a_row_of_another_lane_does_not_stand_in_for_a_missing_one(self):
        extra = rows()[:-1] + [{"name": "catia-07", "result": "PASS", "reason": "-", "done": 3, "total": 3, "failed": []}]
        self.assertEqual(self.d(extra)[0], "RETRY")

    def test_a_pass_with_a_red_reference_after_is_a_retry_hostref(self):
        self.assertEqual(self.d(after="RED"), ("RETRY", "HOSTREF"))

    def test_a_pass_with_an_unrun_reference_after_is_not_a_pass(self):
        self.assertEqual(self.d(after="unrun"), ("RETRY", "HOSTREF"))

    def test_a_pass_with_a_new_host_xid_is_a_fail(self):
        self.assertEqual(self.d(xid=1), ("FAIL", "KAYFABE"))
        self.assertEqual(self.d(xid=3), ("FAIL", "KAYFABE"))

    def test_a_failed_setup_step_is_its_retry_class_even_with_every_row_passed(self):
        self.assertEqual(self.d(abort="MEDIA"), ("RETRY", "MEDIA"))
        self.assertEqual(self.d(abort="TRANSPORT"), ("RETRY", "TRANSPORT"))

    def test_a_setup_failure_never_hides_a_failed_viewset(self):
        self.assertEqual(self.d(rows(**{"creo-04": ("FAIL", "SUBTEST")}), abort="TIME"), ("FAIL", "KAYFABE"))

    def test_the_reason_of_the_first_unrun_viewset_names_the_class(self):
        self.assertEqual(self.d(rows(**{"energy-04": ("NOT_RUN", "NO_TDR")})), ("RETRY", "TRANSPORT"))
        self.assertEqual(self.d(rows(**{"energy-04": ("NOT_RUN", "GATHER")})), ("RETRY", "OPERATOR"))

    def test_the_last_row_of_a_name_counts(self):
        rs = rows(**{"energy-04": ("NOT_RUN", "GATHER")}) + [{"name": "energy-04", "result": "PASS", "reason": "-", "done": 3, "total": 3, "failed": []}]
        self.assertEqual(self.d(rs), ("PASS", "NONE"))

    def test_both_needs_all_nine(self):
        self.assertEqual(self.d(rows(lane="both"), "both"), ("PASS", "NONE"))
        self.assertEqual(self.d(rows(lane="both")[:-1], "both")[0], "RETRY")

    def test_an_unknown_abort_class_is_not_a_class(self):
        self.assertEqual(self.d(abort="WHATEVER"), ("PASS", "NONE") if False else self.d(abort="WHATEVER"))
        self.assertNotEqual(self.d(abort="WHATEVER")[0], "PASS")


class Scrub(unittest.TestCase):
    def test_only_status_lines_survive(self):
        raw = text(FIX / "native_catia-07.cli.log")
        out = lane.scrub_cli(raw)
        self.assertIn("[workload: catia-07 / subtest: 06_LoftJets] Completed", out)
        self.assertNotIn("results and logs", out)
        self.assertNotIn("C:\\spec", out)
        self.assertNotIn("Gathering System Details", out)

    def test_a_score_a_path_and_a_model_name_do_not_survive(self):
        raw = "Composite 123.4\n[workload: catia-07 / subtest: 01_A] Completed\nC:\\spec\\results\\x\nGPU: NVIDIA GeForce RTX 4090\n"
        self.assertEqual(lane.scrub_cli(raw), "[workload: catia-07 / subtest: 01_A] Completed\n")

    def test_a_status_line_with_a_score_appended_does_not_survive(self):
        raw = "[workload: catia-07 / subtest: 01_A] Completed score=99.9\n[workload: catia-07 / subtest: 02_B] Completed\n"
        self.assertEqual(lane.scrub_cli(raw), "[workload: catia-07 / subtest: 02_B] Completed\n")

    def test_a_long_line_does_not_survive(self):
        raw = "[workload: catia-07 / subtest: " + "A" * 300 + "] Completed\n"
        self.assertEqual(lane.scrub_cli(raw), "")

    def test_empty_input_gives_empty_output(self):
        self.assertEqual(lane.scrub_cli(""), "")
        self.assertEqual(lane.scrub_cli("nothing here\n"), "")

    def test_the_refusal_notes_survive_but_nothing_else_on_that_line(self):
        raw = "Error: Unable to gather system details: gwpg_sysinfo.exe -s -d\n  Requirement: raytracing\nError: something with a score 5\n"
        out = lane.scrub_cli(raw)
        self.assertIn("Requirement: raytracing", out)
        self.assertNotIn("score", out)


def tar_of(members):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.GNU_FORMAT) as t:
        for name, kind, data in members:
            info = tarfile.TarInfo(name)
            if kind == "dir":
                info.type = tarfile.DIRTYPE
                t.addfile(info)
            elif kind == "file":
                info.size = len(data)
                t.addfile(info, io.BytesIO(data))
            elif kind == "symlink":
                info.type = tarfile.SYMTYPE
                info.linkname = "x"
                t.addfile(info)
            elif kind == "fifo":
                info.type = tarfile.FIFOTYPE
                t.addfile(info)
    return buf.getvalue()


B = "Program Files/SPECviewperf 15/"
TREE = [
    ("Program Files", "dir", b""),
    (B[:-1], "dir", b""),
    (B + "SPECviewperf-CLI.exe", "file", b"cli"),
    (B + "vcredist_x64.exe", "file", b"vc"),
    (B + "viewsets/catia-07-1.0.6", "dir", b""),
    (B + "viewsets/catia-07-1.0.6/a.bin", "file", b"a" * 10),
    (B + "viewsets/catia-07-1.0.6/sub/b.bin", "file", b"b" * 5),
    (B + "viewsets/blender-01-1.0.1", "dir", b""),
    (B + "viewsets/blender-01-1.0.1/c.bin", "file", b"c" * 100),
    (B + "viewsets/unreal_engine-01-1.0.2/d.bin", "file", b"d" * 7),
]


class TarFilter(unittest.TestCase):
    def run_filter(self, members, wanted):
        src, dst = io.BytesIO(tar_of(members)), io.BytesIO()
        with tempfile.TemporaryDirectory() as d:
            man = Path(d) / "m"
            lines = tarfilter.filter_stream(src, dst, set(wanted), man)
            self.assertEqual(man.read_text().splitlines(), lines)
        names = [m.name for m in tarfile.open(fileobj=io.BytesIO(dst.getvalue()))]
        return names, lines

    def test_another_lanes_viewsets_are_dropped_and_the_rest_is_kept(self):
        names, _ = self.run_filter(TREE, ["catia-07", "unreal_engine-01"])
        self.assertIn(B + "SPECviewperf-CLI.exe", names)
        self.assertIn(B + "vcredist_x64.exe", names)
        self.assertIn(B + "viewsets/catia-07-1.0.6/a.bin", names)
        self.assertIn(B + "viewsets/unreal_engine-01-1.0.2/d.bin", names)
        self.assertFalse([n for n in names if "blender-01" in n])

    def test_a_wanted_viewset_directory_keeps_every_file_below_it(self):
        names, _ = self.run_filter(TREE, ["blender-01"])
        self.assertIn(B + "viewsets/blender-01-1.0.1/c.bin", names)
        self.assertFalse([n for n in names if "catia-07" in n or "unreal" in n])

    def test_nothing_wanted_keeps_only_the_tool(self):
        names, lines = self.run_filter(TREE, [])
        self.assertEqual(names, ["Program Files", B[:-1], B + "SPECviewperf-CLI.exe", B + "vcredist_x64.exe"])
        self.assertEqual([l for l in lines if l.startswith("VIEWSET")], [])

    def test_the_manifest_counts_files_and_bytes_per_kept_viewset(self):
        _, lines = self.run_filter(TREE, ["catia-07", "blender-01"])
        self.assertIn("VIEWSET blender-01-1.0.1 files=1 bytes=100", lines)
        self.assertIn("VIEWSET catia-07-1.0.6 files=2 bytes=15", lines)
        self.assertFalse([l for l in lines if "unreal" in l])
        self.assertTrue(lines[-1].startswith("KEPT members="))

    def test_the_dropped_count_is_reported(self):
        _, lines = self.run_filter(TREE, ["catia-07"])
        self.assertRegex(lines[-1], r"dropped=[1-9]")

    def test_a_link_member_is_refused(self):
        with self.assertRaises(tarfilter.Refused):
            self.run_filter(TREE + [(B + "link", "symlink", b"")], ["catia-07"])

    def test_a_fifo_member_is_refused(self):
        with self.assertRaises(tarfilter.Refused):
            self.run_filter(TREE + [(B + "fifo", "fifo", b"")], ["catia-07"])

    def test_an_absolute_path_is_refused(self):
        with self.assertRaises(tarfilter.Refused):
            self.run_filter(TREE + [("/etc/evil", "file", b"x")], ["catia-07"])

    def test_a_dotdot_component_is_refused(self):
        with self.assertRaises(tarfilter.Refused):
            self.run_filter(TREE + [(B + "../evil", "file", b"x")], ["catia-07"])

    def test_a_windows_drive_path_is_refused(self):
        with self.assertRaises(tarfilter.Refused):
            self.run_filter(TREE + [("C:/evil", "file", b"x")], ["catia-07"])

    def test_a_refused_member_inside_a_dropped_viewset_is_still_refused(self):
        with self.assertRaises(tarfilter.Refused):
            self.run_filter(TREE + [(B + "viewsets/blender-01-1.0.1/ln", "symlink", b"")], ["catia-07"])

    def test_the_command_line_filters_a_stream_and_refuses_a_bad_lane(self):
        with tempfile.TemporaryDirectory() as d:
            man = Path(d) / "m"
            p = subprocess.run([sys.executable, "-I", str(SPEC / "tarfilter.py"), "--lane", "laneb", "--manifest", str(man)],
                               input=tar_of(TREE), capture_output=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            names = [m.name for m in tarfile.open(fileobj=io.BytesIO(p.stdout))]
            self.assertIn(B + "viewsets/catia-07-1.0.6/a.bin", names)
            self.assertFalse([n for n in names if "blender" in n])
            bad = subprocess.run([sys.executable, "-I", str(SPEC / "tarfilter.py"), "--lane", "lanez", "--manifest", str(man)],
                                 input=b"", capture_output=True)
            self.assertEqual(bad.returncode, 2)
            ln = subprocess.run([sys.executable, "-I", str(SPEC / "tarfilter.py"), "--lane", "laneb", "--manifest", str(man)],
                                input=tar_of(TREE + [(B + "x", "symlink", b"")]), capture_output=True)
            self.assertEqual(ln.returncode, 3)

    def test_a_stream_cut_at_a_member_boundary_is_an_error_exit(self):
        with tempfile.TemporaryDirectory() as d:
            raw = tar_of(TREE)
            cut = raw[:1536]  # after the first two header blocks: whole members, no end-of-archive blocks
            p = subprocess.run([sys.executable, "-I", str(SPEC / "tarfilter.py"), "--lane", "lanea", "--manifest", str(Path(d) / "m")],
                               input=cut, capture_output=True)
            self.assertEqual(p.returncode, 4)

    def test_a_truncated_stream_is_an_error_exit(self):
        with tempfile.TemporaryDirectory() as d:
            raw = tar_of(TREE)[:700]
            p = subprocess.run([sys.executable, "-I", str(SPEC / "tarfilter.py"), "--lane", "lanea", "--manifest", str(Path(d) / "m")],
                               input=raw, capture_output=True)
            self.assertNotEqual(p.returncode, 0)


class Cli(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, "-I", str(SPEC / "lane.py"), *args], capture_output=True, text=True)

    def test_ids_and_vars(self):
        self.assertEqual(self.run_cli("ids", "laneb").stdout.strip(), ",".join(lane.lane_ids("laneb", TBL)))
        v = dict(l.split("=", 1) for l in self.run_cli("vars", "lanea").stdout.split())
        self.assertEqual(v["SETTLE_S"], "300")
        self.assertEqual(v["TAIL_S"], "600")
        self.assertEqual(v["MAX_BOOTS"], "3")
        self.assertEqual(v["NEED_GB"], "130")
        self.assertEqual(v["IDS"], ",".join(lane.lane_ids("lanea", TBL)))

    def test_a_bad_lane_is_exit_2(self):
        self.assertEqual(self.run_cli("ids", "lanec").returncode, 2)
        self.assertEqual(self.run_cli("vars", "lanec").returncode, 2)

    def test_timeout_prints_mode_and_seconds(self):
        self.assertEqual(self.run_cli("timeout", "energy-04", "100000").stdout.split(), ["full", "1200"])
        self.assertEqual(self.run_cli("timeout", "energy-04", "100").stdout.split(), ["skip", "0"])

    def test_grade_exit_codes_say_pass_fail_or_not_run(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "c").write_text(cli("catia-07", OK3))
            (d / "e").write_text(END_OK % "catia-07")
            (d / "t").write_text(TDR0)
            ok = self.run_cli("grade", "catia-07", "--cli", str(d / "c"), "--end", str(d / "e"), "--tdr", str(d / "t"), "--scrub-out", str(d / "s"))
            self.assertEqual(ok.returncode, 0)
            self.assertIn("result=PASS", ok.stdout)
            self.assertIn("Completed", (d / "s").read_text())
            (d / "t").write_text(TDR7)
            bad = self.run_cli("grade", "catia-07", "--cli", str(d / "c"), "--end", str(d / "e"), "--tdr", str(d / "t"))
            self.assertEqual(bad.returncode, 1)
            cut = self.run_cli("grade", "catia-07", "--cli", str(d / "c"), "--end", str(d / "e"), "--tdr", str(d / "t"), "--timeout", "cut")
            self.assertEqual(cut.returncode, 3)

    def test_decide_reads_the_rows_file(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "rows"
            f.write_text("".join(lane.row_line(dict(r, failed=[])) + "\n" for r in rows()))
            p = self.run_cli("decide", "lanea", "--rows", str(f), "--before", "green", "--after", "green")
            self.assertEqual(p.stdout.split(), ["PASS", "NONE"])
            p = self.run_cli("decide", "lanea", "--rows", str(f), "--before", "green", "--after", "RED")
            self.assertEqual(p.stdout.split(), ["RETRY", "HOSTREF"])
            p = self.run_cli("decide", "lanea", "--rows", str(f), "--before", "green", "--after", "green", "--abort", "MEDIA")
            self.assertEqual(p.stdout.split(), ["RETRY", "MEDIA"])


if __name__ == "__main__":
    unittest.main()
