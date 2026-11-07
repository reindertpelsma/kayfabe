#!/usr/bin/env python3
"""The SPECviewperf stage of the Vast payload (DRAFT, disabled by default), GPU-free.

FALSIFIER (written before the code): the stage runs without the Worker asking for it (no KF_STAGES slot or no KF_SPEC),
or for the small and middle tiers, or without the Windows gate, or with a lane the Worker did not name; a PASS is graded
with a missing, extra, duplicate, failing or not-run viewset row, with a non-zero exit, a wrong lane, a wrong list size
or two verdicts; a missing pin, a media fault, a transport fault or the time running out is a pass; the stage starts
with too little time for the whole lane; the Windows image is installed on a disk that cannot hold a lane's tree.
"""

import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

import test_vast_payload_v7 as v7

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def payload():
    spec = importlib.util.spec_from_file_location("payload_spec", HERE / "vast_payload.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


M = payload()
TABLE = M.spec_table(ROOT)
LANES = {k: M.spec_names(TABLE, k) for k in ("lanea", "laneb", "both")}


def log(lane="lanea", results=None, verdict="SPECVIEWPERF_STAGE_PASS", rc=0, listed=None, names=None, exit_line=True):
    names = LANES[lane] if names is None else names
    results = results or {}
    out = ["SPEC_STAGE_LIST lane=%s n=%d" % (lane, len(LANES[lane]) if listed is None else listed)]
    for n in names:
        r = results.get(n, "PASS")
        out.append("SPEC_ROW name=%s result=%s subtests=3/3 reason=%s failed=-" % (n, r, "-" if r == "PASS" else "TIME"))
        if r == "FAIL":
            out.append("%s --- FAILED" % n.replace("-", "_"))
    out.append("SPEC_STAGE_COUNTS pass=%d fail=0 not_run=0 of=%d" % (len(names), len(names)))
    out.append(verdict)
    if exit_line:
        out.append("SPECVIEWPERF_STAGE_EXIT rc=%d" % rc)
    return "\n".join(out) + "\n"


class SpecRows(unittest.TestCase):
    m = M

    def rows(self, text, rc=0, lane="lanea"):
        return self.m.specviewperf_rows(text, rc, lane, TABLE)

    def refused(self, text, rc=0, want="specviewperf_retry_operator", lane="lanea"):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.rows(text, rc, lane)
        self.assertEqual(caught.exception.name, want)
        return caught.exception

    def test_a_pass_needs_every_viewset_of_the_lane_exactly_once(self):
        row = self.m.row
        self.assertEqual(self.rows(log("lanea")), [row("specviewperf", 4), row("specviewperf_lanea", 4)])
        self.assertEqual(self.rows(log("laneb"), lane="laneb"), [row("specviewperf", 5), row("specviewperf_laneb", 5)])

    def test_both_lanes_give_a_row_per_lane_and_the_total(self):
        row = self.m.row
        self.assertEqual(
            self.rows(log("both"), lane="both"),
            [row("specviewperf", 9), row("specviewperf_lanea", 4), row("specviewperf_laneb", 5)],
        )

    # One test per way a PASS can be forged; a check that only one test sees is a check one edit removes.
    def test_a_missing_row_is_not_a_pass(self):
        self.refused(log("lanea", names=LANES["lanea"][:-1], listed=4))

    def test_another_missing_row_is_not_a_pass(self):
        self.refused(log("laneb", names=LANES["laneb"][1:], listed=5), lane="laneb")

    def test_an_extra_row_is_not_a_pass(self):
        self.refused(log("lanea", names=LANES["lanea"] + ["catia-07"], listed=4))

    def test_a_row_of_the_other_lane_in_place_of_one_is_not_a_pass(self):
        self.refused(log("lanea", names=LANES["lanea"][:-1] + ["catia-07"], listed=4))

    def test_two_rows_of_the_other_lane_in_place_of_two_are_not_a_pass(self):
        self.refused(log("lanea", names=LANES["lanea"][:-2] + ["catia-07", "maya-07"], listed=4))

    def test_the_rows_of_another_lane_are_not_a_pass_for_lane_b(self):
        self.refused(log("laneb", names=LANES["lanea"] + ["catia-07"], listed=5), lane="laneb")

    def test_a_duplicate_row_is_not_a_pass(self):
        self.refused(log("lanea") + "SPEC_ROW name=creo-04 result=PASS subtests=3/3 reason=- failed=-\n")

    def test_a_failing_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log("lanea", results={"creo-04": "FAIL"}))

    def test_another_failing_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log("laneb", results={"catia-07": "FAIL"}), lane="laneb")

    def test_a_not_run_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log("lanea", results={"enscape-01": "NOT_RUN"}))

    def test_another_not_run_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log("lanea", results={"energy-04": "NOT_RUN"}))

    def test_a_list_for_another_lane_is_not_a_pass(self):
        self.refused(log("lanea").replace("lane=lanea", "lane=laneb"))

    def test_a_list_size_above_the_lane_is_not_a_pass(self):
        self.refused(log("lanea", listed=5))

    def test_a_list_size_below_the_lane_is_not_a_pass(self):
        self.refused(log("lanea", listed=3))

    def test_no_list_line_is_not_a_pass(self):
        self.refused(log("lanea").split("\n", 1)[1])

    def test_a_nonzero_stage_rc_is_not_a_pass(self):
        self.refused(log("lanea"), 1)

    def test_a_timeout_stage_rc_is_not_a_pass(self):
        self.refused(log("lanea"), 124)

    def test_an_exit_line_of_3_is_not_a_pass(self):
        self.refused(log("lanea", rc=3), 0)

    def test_an_exit_line_of_1_is_not_a_pass(self):
        self.refused(log("lanea", rc=1), 0)

    def test_no_exit_line_is_not_a_pass(self):
        self.refused(log("lanea", exit_line=False), 0)

    def test_no_verdict_is_not_a_pass(self):
        self.refused(log("lanea").replace("SPECVIEWPERF_STAGE_PASS\n", ""))

    def test_two_verdicts_are_not_a_pass(self):
        self.refused(log("lanea").replace("SPECVIEWPERF_STAGE_EXIT", "SPECVIEWPERF_STAGE_PASS\nSPECVIEWPERF_STAGE_EXIT"))

    def test_empty_output_is_not_a_pass(self):
        self.refused("")

    def test_the_lane_of_the_request_decides_not_the_lane_in_the_output(self):
        self.refused(log("laneb"), lane="lanea")

    def test_a_failing_stage_is_the_specviewperf_row_with_the_passed_count(self):
        caught = self.refused(
            log("lanea", results={"creo-04": "FAIL"}, verdict="SPECVIEWPERF_STAGE_FAIL", rc=1), 1, want="specviewperf"
        )
        self.assertEqual(caught.rows, [self.m.row("specviewperf_passed", 3)])

    def test_a_fail_verdict_with_a_zero_exit_is_still_a_failing_row(self):
        self.refused(log("lanea", verdict="SPECVIEWPERF_STAGE_FAIL"), 0, want="specviewperf")

    def retry(self, cls, want):
        text = log("lanea", results={"enscape-01": "NOT_RUN"}, verdict="SPECVIEWPERF_STAGE_RETRY class=" + cls, rc=3)
        if not cls:
            text = text.replace(" class=", "")
        self.refused(text, 3, want=want)

    def test_every_retry_class_names_the_cause_and_never_passes(self):
        for cls in ("NOT_BUILT", "MEDIA", "PROVIDER", "TRANSPORT", "TIME", "HOSTREF", "BAR1_EXCEPTION", "OPERATOR", "USAGE"):
            with self.subTest(cls):
                self.retry(cls, "specviewperf_retry_" + cls.lower())

    def test_an_unknown_retry_class_is_operator(self):
        self.retry("X1", "specviewperf_retry_operator")
        self.retry("WHATEVER", "specviewperf_retry_operator")

    def test_a_retry_without_a_class_is_operator(self):
        self.retry("", "specviewperf_retry_operator")

    def test_a_retry_keeps_the_count_of_viewsets_that_passed(self):
        text = log("lanea", results={"enscape-01": "NOT_RUN"}, verdict="SPECVIEWPERF_STAGE_RETRY class=TIME", rc=3)
        caught = self.refused(text, 3, want="specviewperf_retry_time")
        self.assertEqual(caught.rows, [self.m.row("specviewperf_passed", 3)])

    def test_the_wrapper_not_built_answer_is_a_retry_never_a_pass(self):
        self.refused("SPECVIEWPERF_STAGE_RETRY class=NOT_BUILT\nSPECVIEWPERF_STAGE_EXIT rc=3\n", 3, want="specviewperf_retry_not_built")


class SpecRequested(unittest.TestCase):
    m = M

    def test_disabled_by_default_no_request_no_stage(self):
        env = {}
        self.assertIsNone(self.m.spec_requested(env, True, []))
        self.assertIsNone(self.m.spec_requested({}, False, []))

    def test_the_lane_goes_with_the_slot_and_is_popped(self):
        for lane in ("lanea", "laneb", "both"):
            env = {"KF_SPEC": lane}
            self.assertEqual(self.m.spec_requested(env, True, ["specviewperf"]), lane)
            self.assertEqual(env, {})

    def test_a_lane_without_the_slot_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.spec_requested({"KF_SPEC": "lanea"}, True, [])

    def test_the_slot_without_a_lane_is_refused_it_would_silently_run_both(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.spec_requested({}, True, ["specviewperf"])

    def test_an_unknown_lane_is_refused(self):
        for bad in ("lanec", "", "LANEA", "lanea:laneb", "all"):
            with self.subTest(bad), self.assertRaises(self.m.EvidenceError):
                self.m.spec_requested({"KF_SPEC": bad}, True, ["specviewperf"])

    def test_the_stage_without_the_windows_gate_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.spec_requested({"KF_SPEC": "lanea"}, False, ["specviewperf"])
        with self.assertRaises(self.m.EvidenceError):
            self.m.spec_requested({}, False, ["specviewperf"])

    def test_small_and_middle_tiers_cannot_carry_the_slot(self):
        for tier in ("small", "middle", "adhoc"):
            with self.subTest(tier), self.assertRaises(self.m.EvidenceError):
                self.m.tier_requested({"KF_TIER": tier, "KF_STAGES": "specviewperf"}, True)

    def test_the_reporter_validates_kf_spec(self):
        spec = importlib.util.spec_from_file_location("report_for_spec", HERE / "vast_report.py")
        report = importlib.util.module_from_spec(spec)
        import sys

        sys.modules["report_for_spec"] = report
        spec.loader.exec_module(report)
        check = dict((n, c) for n, _, c in report.SPEC)["KF_SPEC"]
        for good in ("lanea", "laneb", "both"):
            self.assertTrue(check(good), good)
        for bad in ("lanec", "LANEA", "lanea:laneb", "1", "lanea ", "x"):
            self.assertFalse(check(bad), bad)
        required = {n: r for n, r, _ in report.SPEC}
        self.assertFalse(required["KF_SPEC"])


class SpecTable(unittest.TestCase):
    m = M

    def read(self, mutate):
        with tempfile.TemporaryDirectory() as d:
            table = json.loads((ROOT / "scripts/bench/specviewperf/lanes.json").read_text())
            mutate(table)
            (Path(d) / "scripts/bench/specviewperf").mkdir(parents=True)
            (Path(d) / "scripts/bench/specviewperf/lanes.json").write_text(json.dumps(table))
            return self.m.spec_table(d)

    def test_the_committed_table_is_accepted(self):
        self.assertEqual(sorted(TABLE["lanes"]), ["lanea", "laneb"])

    def test_a_missing_table_is_a_preflight_failure(self):
        with tempfile.TemporaryDirectory() as d, self.assertRaises(self.m.EvidenceError) as caught:
            self.m.spec_table(d)
        self.assertEqual(caught.exception.name, "preflight")

    def test_a_viewset_in_both_lanes_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.read(lambda t: t["lanes"]["laneb"].append("energy-04"))

    def test_a_viewset_in_no_lane_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.read(lambda t: t["lanes"]["lanea"].remove("energy-04"))

    def test_a_third_lane_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.read(lambda t: t["lanes"].update({"laneb2": ["x"]}))

    def test_a_nonpositive_time_constant_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.read(lambda t: t.update(setup_s=0))

    def test_min_remaining_is_the_whole_lane(self):
        self.assertEqual(self.m.spec_min_remaining_ms(TABLE, "lanea"), int(2700 + 1.25 * 5484 + 600) * 1000)
        self.assertEqual(self.m.spec_min_remaining_ms(TABLE, "laneb"), int(2700 + 1.25 * 3746 + 600) * 1000)

    def test_both_needs_more_than_either_lane(self):
        both = self.m.spec_min_remaining_ms(TABLE, "both")
        self.assertGreater(both, self.m.spec_min_remaining_ms(TABLE, "lanea"))
        self.assertGreater(both, self.m.spec_min_remaining_ms(TABLE, "laneb"))

    def test_both_does_not_fit_what_is_left_of_a_300_minute_box_after_the_windows_gate(self):
        # INFERRED from the measured 53 minutes to the gate pass: the whole tree needs more than the box has left
        self.assertGreater(self.m.spec_min_remaining_ms(TABLE, "both"), (300 - 75) * 60000)


class SpecStageInThePayload(unittest.TestCase):
    """The stage through main(): order, tiers, refusals, failures; the runner is the v7 fixture's fake."""

    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def release(self, spec="lanea", stages="specviewperf", runs=None, tier="large", windows="1", logs=None, soft_ms=300 * 60000, extra=None):
        env = {"KF_TIER": tier}
        if runs is not None:
            env["KF_WINDOWS_RUNS"] = runs
        if stages is not None:
            env["KF_STAGES"] = stages
        if spec is not None:
            env["KF_SPEC"] = spec
        env.update(extra or {})
        self.commands = []
        self.disks = []
        original = self.m.watched

        def spy(command, root, name):
            if name.endswith("_specviewperf"):
                self.commands.append(list(command))
            return original(command, root, name)

        def probe(environ):
            self.disks.append(environ.get("WINVM_DISK_GB"))

        with patch.object(self.m, "watched", side_effect=spy):
            return self._main_fixture(windows=windows, soft_ms=soft_ms, extra_env=env, stage_logs=logs or {}, probe=probe)

    def test_the_stage_runs_last_after_the_windows_gate_with_its_lane(self):
        rc, result, calls = self.release(logs={"specviewperf": (log("lanea"), 0)})
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-2:], ["windows_gate", "specviewperf"])
        self.assertIn({"name": "specviewperf", "pass": 4, "fail": 0, "skip": 0}, result["suites"])
        self.assertIn({"name": "specviewperf_lanea", "pass": 4, "fail": 0, "skip": 0}, result["suites"])
        self.assertEqual(result["verdict"], "PASS")

    def test_the_stage_runs_after_the_second_windows_run_too(self):
        rc, result, calls = self.release(runs="2", logs={"specviewperf": (log("lanea"), 0), "windows_gate_2": (v7.WINDOWS_PASS, 0)})
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-3:], ["windows_gate", "windows_gate_2", "specviewperf"])

    def test_the_command_carries_the_lane_the_deadline_and_the_wrapper(self):
        before = int(time.time())
        self.release(spec="laneb", logs={"specviewperf": (log("laneb"), 0)})
        self.assertEqual(len(self.commands), 1)
        cmd = self.commands[0]
        i = cmd.index("env")
        self.assertEqual(cmd[i + 1], "KF_SPEC_LANE=laneb")
        deadline = int(cmd[i + 2].split("=", 1)[1])
        self.assertTrue(cmd[i + 2].startswith("KF_SPEC_DEADLINE_EPOCH="))
        # the soft deadline minus the margin: 300 minutes ahead, minus two minutes
        self.assertAlmostEqual(deadline, before + 300 * 60 - 120, delta=30)
        self.assertEqual(Path(cmd[i + 4]).name, "vast_stage_specviewperf.sh")
        self.assertEqual(cmd[-1], "/root/prov")

    def test_the_command_names_the_lane_for_every_lane(self):
        for lane in ("lanea", "laneb", "both"):
            with self.subTest(lane):
                self.release(spec=lane, logs={"specviewperf": (log(lane), 0)})
                cmd = self.commands[-1]
                self.assertIn("KF_SPEC_LANE=" + lane, cmd)
                self.assertEqual(len([c for c in cmd if c.startswith("KF_SPEC_LANE=")]), 1)

    def test_the_stage_deadline_keeps_a_margin_before_the_soft_deadline(self):
        before = int(time.time())
        self.release(spec="laneb", soft_ms=200 * 60000, logs={"specviewperf": (log("laneb"), 0)})
        cmd = self.commands[-1]
        deadline = int(cmd[cmd.index("env") + 2].split("=", 1)[1])
        soft = before + 200 * 60
        self.assertLessEqual(deadline, soft - 60)
        self.assertGreater(deadline, soft - 600)

    def test_the_lane_is_not_a_pass_for_the_other_lane(self):
        rc, result, _ = self.release(spec="lanea", logs={"specviewperf": (log("laneb"), 0)})
        self.assertEqual(rc, 1)
        self.assertIn({"name": "specviewperf_retry_operator", "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_the_gate_installs_windows_on_a_disk_that_holds_a_lane(self):
        self.release(logs={"specviewperf": (log("lanea"), 0)})
        self.assertTrue(self.disks)
        self.assertEqual(set(self.disks), {"220"})

    def test_every_lane_sizes_the_disk_for_the_windows_gate(self):
        for lane in ("laneb", "both"):
            with self.subTest(lane):
                self.release(spec=lane, soft_ms=400 * 60000, logs={"specviewperf": (log(lane), 0)})
                self.assertTrue(self.disks)
                self.assertEqual(set(self.disks), {"220"})

    def test_the_second_windows_run_is_sized_too(self):
        self.release(runs="2", logs={"specviewperf": (log("lanea"), 0), "windows_gate_2": (v7.WINDOWS_PASS, 0)})
        self.assertGreaterEqual(len(self.disks), 6)
        self.assertEqual(set(self.disks), {"220"})

    def test_an_explicit_disk_size_wins(self):
        self.release(logs={"specviewperf": (log("lanea"), 0)}, extra={"WINVM_DISK_GB": "300"})
        self.assertEqual(set(self.disks), {"300"})

    def test_without_the_stage_the_disk_size_is_not_touched(self):
        self.release(spec=None, stages=None)
        self.assertEqual(set(self.disks), {None})

    def test_the_stage_does_not_start_without_time_for_the_whole_lane(self):
        # lane A needs 169 minutes
        rc, result, calls = self.release(soft_ms=150 * 60000)
        self.assertEqual(rc, 1)
        self.assertNotIn("specviewperf", calls)
        self.assertIn({"name": "specviewperf_retry_time", "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_lane_b_starts_with_less_time_than_lane_a_needs(self):
        rc, result, calls = self.release(spec="laneb", soft_ms=150 * 60000, logs={"specviewperf": (log("laneb"), 0)})
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-1], "specviewperf")

    def test_both_lanes_never_start_with_the_time_a_300_minute_box_has_left(self):
        rc, result, calls = self.release(spec="both", soft_ms=225 * 60000)
        self.assertNotIn("specviewperf", calls)
        self.assertIn({"name": "specviewperf_retry_time", "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_a_failed_viewset_is_the_specviewperf_failing_row(self):
        text = log("lanea", results={"creo-04": "FAIL"}, verdict="SPECVIEWPERF_STAGE_FAIL", rc=1)
        rc, result, _ = self.release(logs={"specviewperf": (text, 1)})
        self.assertEqual((rc, result["verdict"]), (1, "FAIL"))
        self.assertIn({"name": "specviewperf", "pass": 0, "fail": 1, "skip": 0}, result["suites"])
        self.assertIn({"name": "specviewperf_passed", "pass": 3, "fail": 0, "skip": 0}, result["suites"])

    def test_every_retry_is_a_failing_retry_row(self):
        for cls in ("MEDIA", "TRANSPORT", "TIME", "PROVIDER"):
            with self.subTest(cls):
                text = "SPECVIEWPERF_STAGE_RETRY class=%s\nSPECVIEWPERF_STAGE_EXIT rc=3\n" % cls
                rc, result, _ = self.release(logs={"specviewperf": (text, 3)})
                self.assertEqual((rc, result["verdict"]), (1, "FAIL"))
                self.assertIn({"name": "specviewperf_retry_" + cls.lower(), "pass": 0, "fail": 1, "skip": 0}, result["suites"])

    def test_refused_requests_start_no_stage(self):
        for kwargs in (
            dict(spec=None),  # the slot without a lane
            dict(stages=None),  # a lane without the slot
            dict(spec="lanec"),
            dict(windows="0"),
            dict(tier="middle"),
            dict(tier="small"),
            dict(stages="specviewperf:specviewperf"),
        ):
            with self.subTest(**kwargs):
                rc, result, calls = self.release(**kwargs)
                self.assertEqual((rc, calls), (1, []))
                self.assertEqual(result["verdict"], "FAIL")

    def test_kf_spec_is_popped_before_any_stage_runs(self):
        # the v7 fixture's runner asserts KF_SPEC is not in the environment of any stage
        rc, result, _ = self.release(logs={"specviewperf": (log("lanea"), 0)})
        self.assertEqual(rc, 0, result)

    def test_the_tier_line_names_the_lane(self):
        import io
        from contextlib import redirect_stdout

        buf = io.StringIO()
        with redirect_stdout(buf):
            self.release(spec="laneb", logs={"specviewperf": (log("laneb"), 0)})
        self.assertRegex(buf.getvalue(), r"KF_PAYLOAD_TIER .* stages=specviewperf spec=laneb")

    def test_without_a_request_the_tier_line_says_none_and_no_stage_runs(self):
        import io
        from contextlib import redirect_stdout

        buf = io.StringIO()
        with redirect_stdout(buf):
            rc, result, calls = self.release(spec=None, stages=None, runs="2", logs={"windows_gate_2": (v7.WINDOWS_PASS, 0)})
        self.assertIn(" spec=none", buf.getvalue())
        self.assertNotIn("specviewperf", calls)
        self.assertEqual(rc, 0)

    def test_the_stage_is_disabled_for_the_default_run(self):
        rc, result, calls = self._main_fixture(windows="1", soft_ms=300 * 60000)
        self.assertEqual(rc, 0)
        self.assertNotIn("specviewperf", calls)
        self.assertNotIn("specviewperf", [r["name"] for r in result["suites"]])


class SpecIsGatedLikeTheWorker(unittest.TestCase):
    def test_the_slot_is_plumbing_and_the_worker_sends_no_stage_by_default(self):
        defaults = json.loads((ROOT / "ci/vast-broker/defaults.json").read_text())
        self.assertEqual(defaults["releaseStages"], [])
        self.assertEqual(defaults["stageSlots"]["specviewperf"], {"tiers": ["large"], "state": "PLUMBING"})
        self.assertEqual(M.STAGE_SLOTS["specviewperf"], "PLUMBING")


if __name__ == "__main__":
    unittest.main()
