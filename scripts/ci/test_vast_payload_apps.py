#!/usr/bin/env python3
"""The Linux apps stage of the Vast payload (DRAFT, disabled by default), GPU-free.

FALSIFIER (written before the code): the stage runs while APPS_STAGE_ENABLED is False, or for the
small/adhoc/unknown tier, or is selected through KF_STAGES; a PASS is graded with a missing, extra,
duplicate or failing app row, with a non-zero exit or with two verdicts; a missing bundle or an
exhausted budget is a pass; the shell lane prints a row for an app that never ran, hangs past its
budget, or writes a verdict before every row.
"""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import test_vast_payload_v7 as v7

HERE = Path(__file__).resolve().parent
GATE = HERE.parent / "bench/box/apps_gate.sh"
NAMES = ["nvidia_smi", "deviceQuery", "vectorAdd"]


def payload():
    spec = importlib.util.spec_from_file_location("payload_apps", HERE / "vast_payload.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def log(names=NAMES, results=None, verdict="APPS_STAGE_PASS", rc=0, listed=None):
    results = results or {}
    out = [
        "APPS_STAGE_LIST n=%d budget_s=3000 per_boot=100"
        % (len(names) if listed is None else listed)
    ]
    for n in names:
        out.append("APP_TRY app=%s verdict=PASS rc=0 secs=1" % n)
    for n in names:
        r = results.get(n, "PASS")
        out.append("APP_ROW name=%s os=linux result=%s secs=3 guest_xid=0 note=-" % (n, r))
        if r == "FAIL":
            out.append("%s --- FAILED" % n)
    out.append(verdict)
    out.append("APPS_STAGE_EXIT rc=%d" % rc)
    return "\n".join(out) + "\n"


class AppsRows(unittest.TestCase):
    m = payload()

    def test_pass_needs_every_gate_app_exactly_once(self):
        self.assertEqual(self.m.apps_rows(log(), 0, NAMES), [self.m.row("apps", 3)])

    def refused(self, text, rc, want="apps_retry_operator"):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.apps_rows(text, rc, NAMES)
        self.assertEqual(caught.exception.name, want)

    # One test per way a PASS can be forged; a check that only one test sees is a check one edit removes.
    def test_another_app_in_place_of_a_gate_app_is_not_a_pass(self):
        self.refused(log(["nvidia_smi", "deviceQuery", "clpeak"]), 0)

    def test_another_app_in_place_of_a_different_gate_app_is_not_a_pass(self):
        self.refused(log(["nvidia_smi", "gpu_burn", "vectorAdd"]), 0)

    def test_a_missing_row_is_not_a_pass(self):
        self.refused(log(NAMES[:2], listed=3), 0)

    def test_an_extra_row_is_not_a_pass(self):
        self.refused(log(NAMES + ["clpeak"], listed=3), 0)

    def test_a_duplicate_row_is_not_a_pass(self):
        dup = "APP_ROW name=vectorAdd os=linux result=PASS secs=3 guest_xid=0 note=-\n"
        self.refused(log(NAMES) + dup, 0)

    def test_a_failing_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log(results={"vectorAdd": "FAIL"}), 0)

    def test_another_failing_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log(results={"nvidia_smi": "FAIL"}), 0)

    def test_a_not_run_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log(results={"vectorAdd": "NOT_RUN"}), 0)

    def test_another_not_run_row_under_a_pass_verdict_is_not_a_pass(self):
        self.refused(log(results={"deviceQuery": "NOT_RUN"}), 0)

    def test_a_list_size_above_the_gate_list_is_not_a_pass(self):
        self.refused(log(listed=4), 0)

    def test_a_list_size_below_the_gate_list_is_not_a_pass(self):
        self.refused(log(listed=2), 0)

    def test_a_nonzero_stage_rc_is_not_a_pass(self):
        self.refused(log(), 1)

    def test_a_timeout_stage_rc_is_not_a_pass(self):
        self.refused(log(), 124)

    def test_an_exit_line_of_3_is_not_a_pass(self):
        self.refused(log(rc=3), 0)

    def test_an_exit_line_of_1_is_not_a_pass(self):
        self.refused(log(rc=1), 0)

    def test_no_verdict_is_not_a_pass(self):
        self.refused(log().replace("APPS_STAGE_PASS\n", ""), 0)

    def test_two_verdicts_are_not_a_pass(self):
        self.refused(log().replace("APPS_STAGE_EXIT", "APPS_STAGE_PASS\nAPPS_STAGE_EXIT"), 0)

    def test_no_exit_line_is_not_a_pass(self):
        self.refused(log().replace("APPS_STAGE_EXIT rc=0\n", ""), 0)

    def test_empty_output_is_not_a_pass(self):
        self.refused("", 0)

    def test_a_failing_gate_app_is_the_apps_row_with_the_passed_count(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.apps_rows(
                log(results={"vectorAdd": "FAIL"}, verdict="APPS_STAGE_FAIL", rc=1), 1, NAMES
            )
        self.assertEqual(caught.exception.name, "apps")
        self.assertEqual(caught.exception.rows, [self.m.row("apps_passed", 2)])

    def retry(self, cls, want):
        text = log(results={"vectorAdd": "NOT_RUN"}, verdict="APPS_STAGE_RETRY class=" + cls, rc=3)
        if not cls:
            text = text.replace(" class=", "")
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.apps_rows(text, 3, NAMES)
        self.assertEqual(caught.exception.name, want)

    def test_retry_not_built_names_the_cause_and_never_passes(self):
        self.retry("NOT_BUILT", "apps_retry_media")

    def test_retry_time_names_the_cause_and_never_passes(self):
        self.retry("TIME", "apps_retry_time")

    def test_retry_box_names_the_cause_and_never_passes(self):
        self.retry("BOX", "apps_retry_box")

    def test_an_unknown_retry_class_is_operator(self):
        self.retry("X1", "apps_retry_operator")

    def test_a_retry_without_a_class_is_operator(self):
        self.retry("", "apps_retry_operator")

    def test_fail_verdict_with_a_zero_exit_is_not_an_apps_failure(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.apps_rows(log(verdict="APPS_STAGE_FAIL", rc=0), 0, NAMES)
        self.assertEqual(caught.exception.name, "apps_retry_operator")


class AppsGateList(unittest.TestCase):
    m = payload()

    def read(self, text, tier="large", name="gate_list.txt"):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "scripts/apps").mkdir(parents=True)
            (Path(d) / "scripts/apps" / name).write_text(text)
            return self.m.apps_gate_list(d, tier)

    def test_comments_and_blank_lines_are_ignored(self):
        self.assertEqual(
            self.read("# why\nvectorAdd\n\n  deviceQuery  \n"), ["vectorAdd", "deviceQuery"]
        )

    def test_empty_duplicate_or_odd_names_are_refused(self):
        for text in ("", "# only\n", "a\na\n", "a b\n", "../x\n", "x" * 49 + "\n"):
            with self.subTest(text), self.assertRaises(self.m.EvidenceError):
                self.read(text)

    def test_the_middle_tier_reads_its_own_file(self):
        self.assertEqual(self.read("a\n", "middle", "gate_list_middle.txt"), ["a"])
        with self.assertRaises(FileNotFoundError):
            self.read("a\n", "middle", "gate_list.txt")

    def test_tiers_map_to_the_list_and_the_lane_argument(self):
        self.assertEqual(self.m.APPS_LISTS["large"], ("gate_list.txt", "full"))
        self.assertEqual(self.m.APPS_LISTS["middle"], ("gate_list_middle.txt", "middle"))
        self.assertEqual(set(self.m.APPS_LISTS), set(self.m.APPS_TIERS))

    def committed(self, tier):
        return self.m.apps_gate_list(HERE.parents[1], tier)

    def test_the_committed_lists_name_known_apps(self):
        known = subprocess.run(
            ["bash", str(HERE.parent / "apps/run_apps.sh"), "x", "list"],
            env={"PATH": os.environ["PATH"], "APPS_ROOT": tempfile.mkdtemp()},
            capture_output=True,
            text=True,
            check=True,
        ).stdout.split()
        for tier in ("large", "middle"):
            names = self.committed(tier)
            self.assertTrue(set(names) <= set(known), (tier, sorted(set(names) - set(known))))
            self.assertNotIn("geekbench_gpu", names)  # docs/DOWNLOADS.md: not for scripted use

    def test_the_middle_list_is_the_full_list_without_the_slow_apps(self):
        full, middle = self.committed("large"), self.committed("middle")
        self.assertTrue(set(middle) < set(full))
        self.assertEqual([n for n in full if n in middle], middle)  # same order
        self.assertEqual(set(full) - set(middle), {"gpu_burn", "vkpeak", "clpeak"})

    def test_no_known_defect_is_in_a_gate_list(self):
        # the four UVM demand-paging apps fail in the guest alone (traces/apps_linux_20261107/)
        for tier in ("large", "middle"):
            self.assertFalse(
                {
                    "UnifiedMemoryStreams",
                    "UnifiedMemoryPerf",
                    "conjugateGradientUM",
                    "attach_verify",
                }
                & set(self.committed(tier))
            )


class AppsRealOutput(unittest.TestCase):
    """The draft lane's real output on an RTX 4090 box (traces/apps_linux_20261107/, run g1 and g2,
    2026-11-07) must grade exactly as the stage's contract says."""

    m = payload()
    TRACE = HERE.parents[1] / "traces/apps_linux_20261107"

    def case(self, tier, name):
        text = (self.TRACE / name).read_text()
        return text, self.m.apps_gate_list(HERE.parents[1], tier)

    def test_the_real_g1_stage_output_grades_as_a_pass(self):
        text, names = self.case("large", "g1/gate_full.out")
        self.assertEqual(self.m.apps_rows(text, 0, names), [self.m.row("apps", len(names))])
        self.assertEqual(len(names), 66)

    def test_the_real_g2_stage_output_grades_as_a_pass(self):
        text, names = self.case("middle", "g2/gate_mid.out")
        self.assertEqual(self.m.apps_rows(text, 0, names), [self.m.row("apps", len(names))])
        self.assertEqual(len(names), 63)

    def test_the_real_g3_output_under_the_xid_watch_wrapper_is_clean_and_a_pass(self):
        text, names = self.case("middle", "g3/gate_watched.out")
        self.assertEqual(self.m.watch_delta(text, 0), [])  # 8 host Xid before, 8 after: none new
        self.assertEqual(self.m.apps_rows(text, 0, names), [self.m.row("apps", len(names))])

    def test_a_real_failing_gate_app_under_the_wrapper_is_a_failed_run_never_a_pass(self):
        # g4: a list of vectorAdd, UnifiedMemoryPerf (a known defect), deviceQuery. The stage prints
        # APPS_STAGE_FAIL; the wrapper saw two NEW host Xid 31 and exits 6; no Xid was allowed.
        text = (self.TRACE / "g4/gate_fail.out").read_text()
        names = ["vectorAdd", "UnifiedMemoryPerf", "deviceQuery"]
        with self.assertRaises(self.m.EvidenceError) as bare:
            self.m.apps_rows(text, 1, names)
        self.assertEqual(bare.exception.name, "apps")
        self.assertEqual(bare.exception.rows, [self.m.row("apps_passed", 2)])
        with tempfile.TemporaryDirectory() as d:
            log, allowed = Path(d) / "watch", Path(d) / "allowed"
            log.write_text(text)
            allowed.write_text("")
            with patch.dict(os.environ, {"KF_PAYLOAD_XID_ALLOWED": str(allowed)}):
                with self.assertRaises(self.m.EvidenceError) as caught:
                    self.m.stage_grade(
                        None, None, log, lambda rc: self.m.apps_rows(text, rc, names), 6
                    )
        self.assertEqual(caught.exception.name, "host_xid")
        self.assertEqual(caught.exception.rows, [self.m.row("apps_passed", 2)])

    def test_the_middle_output_is_not_a_pass_for_the_full_list(self):
        text, _ = self.case("middle", "g2/gate_mid.out")
        with self.assertRaises(self.m.EvidenceError):
            self.m.apps_rows(text, 0, self.m.apps_gate_list(HERE.parents[1], "large"))

    def test_one_flipped_row_of_the_real_output_is_not_a_pass(self):
        text, names = self.case("large", "g1/gate_full.out")
        flipped = text.replace(
            " name=vkpeak os=linux result=PASS", " name=vkpeak os=linux result=FAIL", 1
        )
        self.assertNotEqual(flipped, text)
        with self.assertRaises(self.m.EvidenceError):
            self.m.apps_rows(flipped, 0, names)

    def test_one_dropped_row_of_the_real_output_is_not_a_pass(self):
        text, names = self.case("large", "g1/gate_full.out")
        dropped = "".join(
            s for s in text.splitlines(True) if not s.startswith("APP_ROW name=clpeak ")
        )
        self.assertNotEqual(dropped, text)
        with self.assertRaises(self.m.EvidenceError):
            self.m.apps_rows(dropped, 0, names)

    def test_no_row_of_the_real_output_leaks_a_gpu_uuid_or_an_address(self):
        import re

        text = (self.TRACE / "g1/gate_full.out").read_text()
        self.assertIsNone(re.search(r"GPU-[0-9a-f]{8}-", text))
        self.assertIsNone(re.search(r"\b(?:\d{1,3}\.){3}\d{1,3}\b", text))


class AppsRequested(unittest.TestCase):
    m = payload()

    def test_disabled_by_default_for_every_tier(self):
        self.assertIs(self.m.APPS_STAGE_ENABLED, False)
        for tier in ("small", "middle", "large", "adhoc", "unknown"):
            self.assertFalse(self.m.apps_requested(tier))

    def test_enabled_runs_middle_and_large_only(self):
        with patch.object(self.m, "APPS_STAGE_ENABLED", True):
            got = {
                t: self.m.apps_requested(t)
                for t in ("small", "middle", "large", "adhoc", "unknown")
            }
        self.assertEqual(
            got, {"small": False, "middle": True, "large": True, "adhoc": False, "unknown": False}
        )

    def test_kf_stages_cannot_select_it(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.tier_requested({"KF_TIER": "large", "KF_STAGES": "apps"}, True)


class AppsStageInThePayload(unittest.TestCase):
    """The stage through main(): order, tiers, failures; the runner is the v7 fixture's fake."""

    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def run_tier(self, tier, enabled=True, logs=None, windows="0", soft_ms=300 * 60000):
        env = {"KF_TIER": tier}
        self.listed = []
        self.commands = []
        original = self.m.watched

        def spy(command, root, name):
            if str(command[1]).endswith("apps_gate.sh"):
                self.commands.append(list(command))
            return original(command, root, name)

        with (
            patch.object(self.m, "watched", side_effect=spy),
            patch.object(self.m, "APPS_STAGE_ENABLED", enabled),
            patch.object(
                self.m,
                "apps_gate_list",
                side_effect=lambda root, tier: self.listed.append(tier) or NAMES,
            ),
        ):
            return self._main_fixture(
                windows=windows, soft_ms=soft_ms, extra_env=env, stage_logs=logs or {}
            )

    def test_disabled_by_default_runs_no_apps(self):
        for tier in ("middle", "large"):
            with self.subTest(tier):
                rc, result, calls = self.run_tier(tier, enabled=False)
                self.assertEqual(rc, 0, result)
                self.assertNotIn("apps", calls)

    def test_enabled_middle_and_large_run_it_after_cuda_guest_and_before_windows(self):
        for tier in ("middle", "large"):
            with self.subTest(tier):
                rc, result, calls = self.run_tier(tier, logs={"apps": (log(), 0)}, windows="1")
                self.assertEqual(rc, 0, result)
                self.assertEqual(
                    calls,
                    ["provision", "cuda_host", "merge_bar", "cuda_guest", "apps", "windows_gate"],
                )
                self.assertIn({"name": "apps", "pass": 3, "fail": 0, "skip": 0}, result["suites"])
                self.assertEqual(self.listed, [tier])  # each tier reads ITS list
                self.assertEqual(len(self.commands), 1)
                self.assertEqual(self.commands[0][-1], {"middle": "middle", "large": "full"}[tier])
                self.assertEqual(self.commands[0][-2], "/root/prov")

    def test_the_middle_tier_asks_for_the_middle_list(self):
        rc, result, calls = self.run_tier("middle", logs={"apps": (log(), 0)})
        self.assertEqual(rc, 0, result)
        self.assertEqual(self.listed, ["middle"])

    def test_the_large_tier_asks_for_the_full_list(self):
        rc, result, calls = self.run_tier("large", logs={"apps": (log(), 0)})
        self.assertEqual(rc, 0, result)
        self.assertEqual(self.listed, ["large"])

    def test_the_middle_tier_runs_the_lane_with_the_middle_argument(self):
        self.run_tier("middle", logs={"apps": (log(), 0)})
        self.assertEqual(self.commands[0][-1], "middle")

    def test_small_and_adhoc_never_run_it(self):
        for tier in ("small", "adhoc"):
            with self.subTest(tier):
                rc, result, calls = self.run_tier(tier)
                self.assertEqual(rc, 0, result)
                self.assertNotIn("apps", calls)

    def test_a_failing_gate_app_fails_the_run_and_windows_does_not_start(self):
        text = log(results={"vectorAdd": "FAIL"}, verdict="APPS_STAGE_FAIL", rc=1)
        rc, result, calls = self.run_tier("middle", logs={"apps": (text, 1)}, windows="1")
        self.assertEqual((rc, result["verdict"]), (1, "FAIL"))
        self.assertEqual(calls[-1], "apps")
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["apps"])
        self.assertIn({"name": "apps_passed", "pass": 2, "fail": 0, "skip": 0}, result["suites"])

    def test_a_missing_bundle_is_a_retry_row_never_a_pass(self):
        text = "APPS_STAGE_RETRY class=NOT_BUILT\nAPPS_STAGE_EXIT rc=3\n"
        rc, result, calls = self.run_tier("middle", logs={"apps": (text, 3)})
        self.assertEqual(rc, 1)
        self.assertEqual(
            [r["name"] for r in result["suites"] if r["fail"]], ["apps_retry_media"]
        )

    def short_of_time(self, minutes):
        rc, result, calls = self.run_tier(
            "middle", logs={"apps": (log(), 0)}, soft_ms=minutes * 60000
        )
        self.assertEqual(rc, 1)
        self.assertNotIn("apps", calls)
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["apps_retry_time"])

    def test_30_minutes_left_is_a_retry_row_without_starting_the_stage(self):
        self.short_of_time(30)

    def test_just_under_the_minimum_left_is_a_retry_row_without_starting_the_stage(self):
        self.short_of_time(59)


class AppsGateScript(unittest.TestCase):
    """scripts/bench/box/apps_gate.sh with a fake matrix: the lane's own contract."""

    def run_gate(
        self,
        rows,
        budget="30",
        provisioned=True,
        matrix_body=None,
        list_text="vectorAdd\ndeviceQuery\n",
        tag="ci0abc",
        isolated=(),
    ):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "apps").mkdir()
            if provisioned:
                (root / "apps/bundle.tgz").write_text("x")
                (root / "apps/PROVISIONED").write_text("x")
            (root / "list.txt").write_text(list_text)
            (root / "matrix.sh").write_text(
                matrix_body
                or (
                    "#!/bin/bash\nR=%s/apps/results/$2; mkdir -p $R\n" % root
                    + "".join("echo '%s' >> $R/guest.res\n" % line for line in rows)
                    + "".join("echo '%s' >> $R/guest_isolated.res\n" % line for line in isolated)
                )
            )
            env = {
                "PATH": os.environ["PATH"],
                "KF_APPS_LIST": str(root / "list.txt"),
                "KF_APPS_ROOT": str(root / "apps"),
                "KF_APPS_MATRIX": str(root / "matrix.sh"),
                "KF_APPS_BUDGET_S": budget,
                "KF_APPS_POLL_S": "0.2",
            }
            p = subprocess.run(
                ["bash", str(GATE), tag, str(root / "prov")],
                env=env,
                capture_output=True,
                text=True,
                timeout=120,
            )
            return p.returncode, p.stdout

    @staticmethod
    def res(app, verdict="PASS", secs=2):
        return (
            "APPRES side=guest app=%s verdict=%s rc=0 secs=%d quiet=0 note=- boot=ci0abc_b1 guest_xid=0 kf3_lines=1 kf3_refusals=0"
            % (app, verdict, secs)
        )

    def test_unsupported_is_reported_when_bundle_is_missing(self):
        rc, out = self.run_gate([], provisioned=False)
        self.assertEqual(rc, 3)
        self.assertIn("APP_POLICY name=attach_verify os=linux result=NOT_RUN note=unsupported (UVM)", out)

    def test_unsupported_cannot_be_in_an_override_gate_list(self):
        for name in ("UnifiedMemoryPerf", "attach_verify"):
            rc, out = self.run_gate([], list_text=name + "\n")
            self.assertEqual(rc, 3)
            self.assertIn("APPS_STAGE_RETRY class=BAD_LIST", out)
            self.assertNotIn("APPS_STAGE_START", out)

    def test_unsupported_streams_cannot_be_in_an_override_gate_list(self):
        rc, out = self.run_gate([], list_text="UnifiedMemoryStreams\n")
        self.assertEqual(rc, 3)
        self.assertIn("APPS_STAGE_RETRY class=BAD_LIST", out)
        self.assertNotIn("APPS_STAGE_START", out)

    def test_all_pass(self):
        rc, out = self.run_gate([self.res("vectorAdd"), self.res("deviceQuery")])
        self.assertEqual(rc, 0, out)
        lines = out.splitlines()
        self.assertIn("APPS_STAGE_LIST n=2 budget_s=30 per_boot=100", lines)
        self.assertEqual(sum(x.startswith("APP_POLICY ") for x in lines), 4)
        self.assertEqual(lines[-2:], ["APPS_STAGE_PASS", "APPS_STAGE_EXIT rc=0"])
        self.assertEqual(sum(s.startswith("APP_ROW ") and " result=PASS " in s for s in lines), 2)
        self.assertLess(
            max(i for i, s in enumerate(lines) if s.startswith("APP_ROW")), len(lines) - 2
        )

    def test_a_guest_failure_is_fail_and_names_the_app(self):
        rc, out = self.run_gate([self.res("vectorAdd"), self.res("deviceQuery", "FAIL")])
        self.assertEqual(rc, 1, out)
        self.assertIn("deviceQuery --- FAILED", out)
        self.assertEqual(out.splitlines()[-2:], ["APPS_STAGE_FAIL", "APPS_STAGE_EXIT rc=1"])

    def test_the_isolated_rerun_is_the_verdict_of_record(self):
        # batched FAIL, alone PASS: a boot that degraded, not a failing app; the row says PASS
        rc, out = self.run_gate(
            [self.res("vectorAdd"), self.res("deviceQuery", "FAIL")],
            isolated=[self.res("deviceQuery", "PASS")],
        )
        self.assertEqual(rc, 0, out)
        self.assertEqual(out.splitlines()[-2:], ["APPS_STAGE_PASS", "APPS_STAGE_EXIT rc=0"])
        # and the other way: batched PASS never hides an isolated FAIL
        rc, out = self.run_gate(
            [self.res("vectorAdd"), self.res("deviceQuery")],
            isolated=[self.res("deviceQuery", "FAIL")],
        )
        self.assertEqual(rc, 1, out)

    def test_no_rows_at_all_is_retry_box_not_a_pass(self):
        rc, out = self.run_gate([])
        self.assertEqual(rc, 3, out)
        self.assertIn("APPS_STAGE_RETRY class=BOX", out)
        self.assertEqual(sum(line.startswith("APP_ROW ") and "result=NOT_RUN" in line for line in out.splitlines()), 2)

    def test_budget_exhausted_is_retry_time_with_unrun_rows(self):
        rc, out = self.run_gate([], budget="1", matrix_body="#!/bin/bash\nsleep 60\n")
        self.assertEqual(rc, 3, out)
        self.assertIn("APPS_STAGE_RETRY class=TIME", out)
        self.assertEqual(sum(line.startswith("APP_ROW ") and "result=NOT_RUN" in line for line in out.splitlines()), 2)

    def test_missing_bundle_is_retry_media_and_runs_nothing(self):
        rc, out = self.run_gate([self.res("vectorAdd"), self.res("deviceQuery")], provisioned=False)
        self.assertEqual(rc, 3, out)
        self.assertIn("APPS_STAGE_RETRY class=MEDIA", out)
        self.assertNotIn("APP_ROW", out)

    def test_bad_lists_are_refused(self):
        for text, cls in (
            ("", "NO_LIST"),
            ("# only\n", "NO_LIST"),
            ("nosuchapp\n", "BAD_LIST"),
            ("a b\n", "BAD_LIST"),
        ):
            with self.subTest(text):
                rc, out = self.run_gate([], list_text=text)
                self.assertEqual(rc, 3, out)
                self.assertIn("APPS_STAGE_RETRY class=" + cls, out)

    def test_an_unknown_list_name_is_usage(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run(
                ["bash", str(GATE), "ci0abc", d, "everything"], capture_output=True, text=True
            )
        self.assertEqual(p.returncode, 2, p.stdout)
        self.assertIn("class=USAGE", p.stdout)

    def test_a_bad_tag_is_usage(self):
        rc, out = self.run_gate([], tag="bad tag;")
        self.assertEqual(rc, 2, out)
        self.assertIn("class=USAGE", out)


if __name__ == "__main__":
    unittest.main()
