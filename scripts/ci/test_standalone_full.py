#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the standalone FULL run (2026-11-17): scripts/ci/standalone_full.py (the manifest, the refusals by name, the record), its wiring in
vast_payload.py (`--full`, the stages `apps_discovery` and `windows_open_rows`), scripts/ci/apps_discovery_stage.py and the report validators. No VM, no GPU, no network."""
import contextlib
import io
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import types
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "scripts"))
import apps_discovery_stage as ads  # noqa: E402
import payload_instruction as pi  # noqa: E402
import profile_resources  # noqa: E402
import standalone_full as sf  # noqa: E402
import vast_payload  # noqa: E402
import vast_report  # noqa: E402
from kfci import apps_gate  # noqa: E402

SHA = "1234abcd" * 5


def instruction(**kw):
    base = dict(tier="large", windows=True, windows_runs=2, stages=("specviewperf",), spec="both", winapps="mid", linux_apps="middle", rehearsal=True,
                propimages=({"label": "imga"},), propimages_strict=True, source="default")
    base.update(kw)
    return pi.Instruction(**base)


def args(**kw):
    base = dict(only=[], skip=[], matlab_image="/x/matlab.zip", matlab_driver=None, full=True, instruction="/x/i.json")
    base.update(kw)
    return types.SimpleNamespace(**base)


class ManifestTests(unittest.TestCase):
    def test_every_manifest_stage_is_a_payload_stage_and_the_new_ones_are_manual_only(self):
        source = (HERE / "vast_payload.py").read_text()
        manual, flags = profile_resources.manual_only(source)
        names = profile_resources.stage_names(source) | set(manual)
        for stage in sf.FULL_STAGES:
            self.assertIn(stage, names, stage)
        for stage in ("apps_discovery", "windows_open_rows"):
            self.assertIn(stage, manual)
            self.assertIs(flags[manual[stage]], False)
        self.assertEqual(len(set(sf.FULL_STAGES)), len(sf.FULL_STAGES))

    def test_the_full_plan_is_the_manifest_in_the_manifest_order(self):
        plan = vast_payload.plan_stage_names(instruction(), (), True)
        self.assertEqual(plan, list(sf.FULL_STAGES))

    def test_the_default_plan_does_not_change(self):
        # a Worker tier that does not ask for --full plans what it planned before: none of the manual-only stages
        plan = vast_payload.plan_stage_names(instruction())
        for stage in ("apps_discovery", "windows_open_rows", "install_flow", "propimage_apps_b", "matlab"):
            self.assertNotIn(stage, plan)
        self.assertEqual(plan, ["provision", "cuda_host", "merge_bar", "cuda_guest", "apps", "kfml", "windows_gate", "windows_apps", "windows_gate_2", "propimage_apps", "specviewperf"])

    def test_only_names_a_new_stage_and_a_small_tier_never_gets_one(self):
        self.assertIn("apps_discovery", vast_payload.plan_stage_names(instruction(), ["apps_discovery"]))
        self.assertIn("windows_open_rows", vast_payload.plan_stage_names(instruction(), ["windows_open_rows"]))
        for tier in ("small", "middle", "adhoc"):
            plan = vast_payload.plan_stage_names(instruction(tier=tier), (), True)
            self.assertNotIn("apps_discovery", plan)
            self.assertNotIn("windows_open_rows", plan)

    def test_a_manual_stage_is_planned_for_the_large_tier_only_whatever_names_it(self):
        for stage in ("apps_discovery", "windows_open_rows", "install_flow", "propimage_apps_b", "matlab"):
            for tier in ("small", "middle", "adhoc"):
                with self.subTest(stage=stage, tier=tier):
                    self.assertNotIn(stage, vast_payload.plan_stage_names(instruction(tier=tier, propimages=(), propimages_strict=False), [stage]))

    def test_by_design_names_are_manifest_stages(self):
        for stage in sf.BY_DESIGN_NOT_RUN:
            self.assertIn(stage, sf.FULL_STAGES)

    def test_only_the_open_windows_stage_asks_for_an_online_guest(self):
        source = (HERE / "vast_payload.py").read_text()
        self.assertEqual(source.count('"WINVM_NET=online"'), 1)
        self.assertNotIn("WINVM_NET=online", (HERE / "standalone_full.py").read_text().replace("WINVM_NET=online;", ""))

    def test_the_result_row_bound_fits_one_full_run(self):
        rows = {"provision": 1, "cuda_host": 1, "merge_bar": 6, "cuda_guest": 1, "apps": 2, "apps_discovery": 1, "kfml": 1, "install_flow": 1, "windows_gate": 1, "windows_apps": 1,
                "windows_gate_2": 1, "windows_open_rows": 1, "propimage_apps": 1, "propimage_apps_b": 1, "matlab": 1, "specviewperf": 3}
        self.assertEqual(set(rows), set(sf.FULL_STAGES))
        total = sum(rows.values()) + 1                                  # plus the failure row of a red stage
        self.assertGreater(total, 20)                                   # the bound before this change (19 here, 20 in the Worker) could not hold a full run
        self.assertLessEqual(total, vast_report.MAX_SUITE_ROWS)
        worker = (ROOT / "ci/vast-broker/src/results.js").read_text()
        self.assertEqual(int(re.search(r"rows\.length > ([0-9]+)", worker)[1]), vast_report.MAX_SUITE_ROWS)


class RefusalTests(unittest.TestCase):
    def test_a_complete_input_set_has_no_refusal(self):
        self.assertEqual(sf.refusals(instruction(), args(), (), online=True), [])

    def test_each_missing_input_is_named_by_stage_and_input(self):
        cases = {
            "linux_apps": (dict(linux_apps=None), "apps:linux_apps"),
            "windows": (dict(windows=False, windows_runs=1, winapps=None), "windows_gate:windows"),
            "winapps": (dict(winapps=None), "windows_apps:winapps"),
            "gate2": (dict(windows_runs=1), "windows_gate_2:windows_runs_2"),
            "propimages": (dict(propimages=(), propimages_strict=False), "propimage_apps:propimages"),
            "spec": (dict(stages=(), spec=None), "specviewperf:spec_both"),
            "spec_lane": (dict(spec="lanea"), "specviewperf:spec_both"),
        }
        for label, (change, token) in cases.items():
            with self.subTest(label):
                self.assertIn(token, sf.refusals(instruction(**change), args(), (), online=True))

    def test_every_other_tier_is_refused_by_name(self):
        for tier in ("small", "middle", "adhoc"):
            with self.subTest(tier):
                self.assertIn("tier:large", sf.refusals(instruction(tier=tier), args(), (), online=True))

    def test_matlab_needs_an_image_from_the_flag_or_the_instruction(self):
        self.assertIn("matlab:matlab_image", sf.refusals(instruction(), args(matlab_image=None), (), online=True))
        self.assertNotIn("matlab:matlab_image", sf.refusals(instruction(matlab={"rows": []}), args(matlab_image=None), (), online=True))

    def test_the_full_run_is_large_tier_and_strict(self):
        self.assertIn("tier:large", sf.refusals(instruction(tier="middle", matlab=None), args(), (), online=True))
        self.assertIn("propimage_apps:strict", sf.refusals(instruction(propimages_strict=False), args(), (), online=True))

    def test_an_offline_host_names_the_online_windows_rows(self):
        self.assertIn("windows_open_rows:online_host", sf.refusals(instruction(), args(), (), online=False))
        self.assertNotIn("windows_open_rows:online_host", sf.refusals(instruction(), args(), (), online=None))

    def test_a_skip_removes_the_check_and_is_recorded(self):
        self.assertEqual(sf.refusals(instruction(propimages=(), propimages_strict=False), args(), ("propimage_apps",), online=True), [])
        rec = sf.record([s for s in sf.FULL_STAGES if s != "propimage_apps"], ("propimage_apps",))
        self.assertEqual(rec["skipped"], ["propimage_apps"])
        self.assertFalse(rec["complete"])
        self.assertIn("skipped=propimage_apps", sf.line(rec))
        self.assertIn("complete=0", sf.line(rec))

    def test_a_run_without_a_skip_is_complete(self):
        rec = sf.record(list(sf.FULL_STAGES), ())
        self.assertTrue(rec["complete"])
        self.assertEqual(rec["skipped"], [])
        self.assertIn("skipped=none complete=1", sf.line(rec))

    def test_flag_combinations_that_would_make_a_different_run_are_refused(self):
        self.assertEqual(sf.check_flags(["apps"], [], True), "full:with_only")
        self.assertEqual(sf.check_flags([], [], False), "full:needs_instruction_file")
        self.assertEqual(sf.check_flags([], ["nope"], True), "skip:unknown:nope")
        self.assertEqual(sf.check_flags([], ["provision"], True), "skip:prerequisite:provision")
        self.assertEqual(sf.check_flags([], ["merge_bar"], True), "skip:prerequisite:merge_bar")
        self.assertIsNone(sf.check_flags([], ["matlab", "windows_open_rows"], True))
        self.assertEqual(sf.check_flags([], ["windows_gate"], True), "skip:needs:windows_apps:windows_gate")
        self.assertEqual(sf.check_flags([], ["apps"], True), "skip:needs:apps_discovery:apps")
        self.assertIsNone(sf.check_flags([], ["apps", "apps_discovery"], True))

    def test_online_probe_is_one_connect_and_never_raises(self):
        seen = []

        class C:
            def close(self):
                seen.append("closed")

        self.assertTrue(sf.online_capable(lambda target, timeout: C()))
        self.assertEqual(seen, ["closed"])

        def refuse(target, timeout):
            raise OSError("no route")

        self.assertFalse(sf.online_capable(refuse))


FIXTURE = ROOT / "ci/vast-broker/test/fixtures/propimages/ad102-large-slot1-even.instruction.json"


class CommandLineTests(unittest.TestCase):
    def run_main(self, *argv):
        out = io.StringIO()
        old = sys.argv
        sys.argv = ["vast_payload.py", *argv]
        try:
            with contextlib.redirect_stdout(out):
                rc = vast_payload.main()
        finally:
            sys.argv = old
        return rc, out.getvalue()

    def instruction_file(self, **change):
        """The committed Worker golden of image A (a valid strict instruction) with the SPEC lanes added, then `change` on top; a None value removes the key, "NULL" writes a JSON null."""
        doc = json.loads(FIXTURE.read_text())
        doc.update(spec="both", stages=["specviewperf"])
        for key, value in change.items():
            if value is None:
                doc.pop(key, None)
            else:
                doc[key] = None if value == "NULL" else value
        d = tempfile.TemporaryDirectory(prefix="full-test-")
        self.addCleanup(d.cleanup)
        path = Path(d.name) / "i.json"
        path.write_text(json.dumps(doc))
        return str(path)

    def dry(self, *extra, image=True, **change):
        image_flag = ["--matlab-image", "/x.zip"] if image else []
        return self.run_main("--dry-run", "--full", "--instruction", self.instruction_file(**change), *image_flag, *extra)

    def test_a_full_dry_run_with_every_input_prints_the_whole_manifest(self):
        rc, out = self.dry()
        self.assertEqual(rc, 0, out)
        plan = re.search(r"KF_PAYLOAD_PLAN stages=(\S+)", out)[1].split(":")
        self.assertEqual(plan, list(sf.FULL_STAGES))
        self.assertIn("KF_PAYLOAD_FULL manifest=1 stages=16 skipped=none complete=1", out)

    def test_a_dry_run_without_full_plans_none_of_the_manual_stages(self):
        rc, out = self.run_main("--dry-run", "--instruction", self.instruction_file())
        self.assertEqual(rc, 0, out)
        plan = re.search(r"KF_PAYLOAD_PLAN stages=(\S+)", out)[1].split(":")
        for stage in ("apps_discovery", "windows_open_rows", "propimage_apps_b", "matlab", "install_flow"):
            self.assertNotIn(stage, plan)
        self.assertNotIn("KF_PAYLOAD_FULL", out)

    def test_each_missing_input_is_one_named_refusal_line_and_no_plan(self):
        cases = {
            "propimage_apps:propimages": dict(propimages=None, propimages_strict=None),
            "propimage_apps:strict": dict(propimages_strict=False),
            "apps:linux_apps": dict(linux_apps="NULL"),
            "windows_gate_2:windows_runs_2": dict(windows_runs=1),
            "windows_apps:winapps": dict(winapps="NULL"),
            "specviewperf:spec_both": dict(spec="lanea"),
        }
        for token, change in cases.items():
            with self.subTest(token):
                rc, out = self.dry(**change)
                self.assertEqual(rc, 1, out)
                self.assertIn("KF_PAYLOAD_FULL_REFUSED " + token + "\n", out)
                self.assertIn("KF_PAYLOAD_FAIL suite=preflight", out)
                self.assertNotIn("KF_PAYLOAD_PLAN", out)

    def test_a_full_dry_run_without_a_matlab_image_fails_by_name(self):
        rc, out = self.run_main("--dry-run", "--full", "--instruction", self.instruction_file())
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_FULL_REFUSED matlab:matlab_image", out)

    def test_skips_are_printed_and_the_run_says_it_is_not_complete(self):
        rc, out = self.dry("--skip", "matlab", "--skip", "windows_open_rows", image=False)
        self.assertEqual(rc, 0, out)
        self.assertIn("skipped=windows_open_rows,matlab complete=0", out)
        plan = re.search(r"KF_PAYLOAD_PLAN stages=(\S+)", out)[1].split(":")
        self.assertNotIn("matlab", plan)
        self.assertNotIn("windows_open_rows", plan)
        self.assertEqual(len(plan), 14)

    def test_a_skip_that_the_run_cannot_do_without_is_refused_by_name(self):
        for flag, token in (("provision", "skip:prerequisite:provision"), ("merge_bar", "skip:prerequisite:merge_bar"),
                            ("windows_gate", "skip:needs:windows_apps:windows_gate"), ("apps", "skip:needs:apps_discovery:apps"), ("nope", "skip:unknown:nope")):
            with self.subTest(flag):
                rc, out = self.dry("--skip", flag)
                self.assertEqual(rc, 1, out)
                self.assertIn("KF_PAYLOAD_FULL_REFUSED " + token, out)

    def test_full_with_only_and_full_without_a_file_are_refused(self):
        rc, out = self.run_main("--dry-run", "--full", "--only", "apps", "--instruction", self.instruction_file())
        self.assertEqual((rc, "KF_PAYLOAD_FULL_REFUSED full:with_only" in out), (1, True))
        rc, out = self.run_main("--dry-run", "--full")
        self.assertEqual((rc, "KF_PAYLOAD_FULL_REFUSED full:needs_instruction_file" in out), (1, True))

    def test_the_online_host_check_names_the_online_windows_rows(self):
        ns = args(only=[], skip=[], instruction="/x/i.json")
        out = io.StringIO()
        with contextlib.redirect_stdout(out), self.assertRaises(vast_payload.EvidenceError):
            vast_payload.full_check(instruction(), ns, list(sf.FULL_STAGES), online=False)
        self.assertIn("KF_PAYLOAD_FULL_REFUSED windows_open_rows:online_host", out.getvalue())
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rec = vast_payload.full_check(instruction(), ns, list(sf.FULL_STAGES), online=True)
        self.assertTrue(rec["complete"])


WIN_GRADE = "WINAPPS_GRADE kind=%s expected=%d pass=%d guest_fail=%d boot_fail=%d harness=%d not_run=%d extra=0\n"


def app_line(app, cls, verdict="PASS"):
    return "WINAPPS_APP app=%s class=%s verdict=%s batched=- alone=- secs=1 tdr=0 wer=0 gsp=- note=-\n" % (app, cls, verdict)


class MatlabInFullRunTests(unittest.TestCase):
    def test_the_full_run_asks_the_matlab_stage_for_zero_skips(self):
        source = (HERE / "vast_payload.py").read_text()
        block = source[source.index('if "matlab" in plan:'):]
        block = block[:block.index('if "specviewperf" in extra_stages:')]
        self.assertRegex(block, r"if args\.full:\s*\n\s*matlab_argv\.append\(\"--strict-rows\"\)")

    def test_the_matlab_block_names_the_full_switch_and_the_flag(self):
        source = (HERE / "vast_payload.py").read_text()
        block = source[source.index('if "matlab" in plan:'):]
        block = block[:block.index('if "specviewperf" in extra_stages:')]
        self.assertIn("if args.full:", block)
        self.assertIn('"--strict-rows"', block)

    def test_the_stage_has_the_flag_and_the_payload_refusal_accepts_it_with_full(self):
        self.assertRegex((HERE / "matlab_stage.py").read_text(), r'add_argument\("--strict-rows", action="store_true"')
        ns = args(only=[], skip=[], matlab_image="/x.zip")
        self.assertIsNone(vast_payload.matlab_refusal(instruction(matlab=None), ns))
        self.assertEqual(vast_payload.matlab_refusal(instruction(matlab=None), args(matlab_image=None)), "NO_IMAGE")
        self.assertIsNone(vast_payload.matlab_refusal(instruction(matlab=None), args(matlab_image=None, skip=["matlab"])))
        self.assertEqual(vast_payload.matlab_refusal(instruction(matlab=None), args(matlab_image="/x.zip", full=False)), "FLAGS_NEED_ONLY_MATLAB")


    def test_the_stage_accepts_the_flag_on_its_command_line(self):
        import subprocess
        done = subprocess.run([sys.executable, "-B", str(HERE / "matlab_stage.py"), "run", "--help"], capture_output=True, text=True, timeout=60)
        self.assertEqual(done.returncode, 0)
        self.assertRegex(done.stdout, r"--strict-rows(?![-\w])")


class WindowsOpenRowsTests(unittest.TestCase):
    ROWS = ("vkglcts_gl46", "vkglcts_vk", "vkglcts_gl_dual_soak", "dxsamples_nbody_async", "dxsamples_execute_indirect", "dxsamples_reserved_resources", "dxsamples_hdr",
            "dxsamples_work_graphs", "dxsamples_multiadapter")

    def log(self, classes, kind="RETRY", exit_rc=3):
        text = "".join(app_line(a, classes.get(a, "pass"), {"pass": "PASS", "guest_fail": "FAIL", "boot_fail": "BOOT_FAIL"}.get(classes.get(a, "pass"), "NOTRUN")) for a in self.ROWS)
        n = lambda c: sum(1 for a in self.ROWS if classes.get(a, "pass") == c)
        text += WIN_GRADE % (kind, len(self.ROWS), n("pass"), n("guest_fail"), n("boot_fail"), n("harness"), n("not_run"))
        return text + "WINAPPS_EXIT rc=%d 2026-11-17T00:00:00+00:00\n" % exit_rc

    def grade(self, classes, rc=3, kind="RETRY"):
        result = {}
        rows = vast_payload.windows_open_rows_rows(self.log(classes, kind, rc), rc, SHA, result)
        return rows, result["windows_open_rows"]

    BY_DESIGN = {"vkglcts_gl46": "harness", "vkglcts_vk": "harness", "dxsamples_multiadapter": "harness"}

    def test_only_the_three_by_design_rows_not_running_passes_and_is_named(self):
        rows, rec = self.grade(dict(self.BY_DESIGN))
        self.assertEqual((rows[0]["name"], rows[0]["pass"], rows[0]["skip"], rows[0]["fail"]), ("windows_open_rows", 6, 3, 0))
        self.assertEqual(rec["not_run_by_design"], ["dxsamples_multiadapter", "vkglcts_gl46", "vkglcts_vk"])
        self.assertEqual((rec["not_run"], rec["breakages"], rec["ran"], rec["passed"]), ([], [], 6, 6))

    def test_a_row_that_ran_and_failed_is_a_listed_breakage_not_a_failure(self):
        rows, rec = self.grade({**self.BY_DESIGN, "dxsamples_hdr": "guest_fail"}, rc=1, kind="FAIL")
        self.assertEqual(rec["breakages"], ["dxsamples_hdr"])
        self.assertEqual((rows[0]["pass"], rows[0]["fail"]), (5, 0))

    def test_any_other_row_that_did_not_run_fails_by_name(self):
        for cls in ("harness", "not_run"):
            with self.subTest(cls):
                with self.assertRaises(vast_payload.EvidenceError) as caught:
                    self.grade({**self.BY_DESIGN, "vkglcts_gl_dual_soak": cls})
                self.assertEqual(caught.exception.name, "windows_open_rows_not_run")

    def test_every_row_failing_to_boot_is_a_failure(self):
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            self.grade({a: "boot_fail" for a in self.ROWS}, rc=1, kind="FAIL")
        self.assertEqual(caught.exception.name, "windows_open_rows")

    def test_every_row_that_ran_and_failed_is_still_only_a_list_of_breakages(self):
        failing = {a: "guest_fail" for a in self.ROWS if a not in self.BY_DESIGN}
        rows, rec = self.grade({**self.BY_DESIGN, **failing}, rc=1, kind="FAIL")
        self.assertEqual((len(rec["breakages"]), rec["passed"], rows[0]["pass"], rows[0]["fail"]), (6, 0, 0, 0))

    def test_the_record_accounts_for_every_expected_row(self):
        for classes in (dict(self.BY_DESIGN), {**self.BY_DESIGN, "dxsamples_hdr": "guest_fail"}):
            rows, rec = self.grade(classes, rc=1 if "dxsamples_hdr" in classes else 3, kind="FAIL" if "dxsamples_hdr" in classes else "RETRY")
            self.assertEqual(rec["ran"] + len(rec["not_run_by_design"]) + len(rec["not_run"]), rec["expected"])

    def test_one_row_outside_the_by_design_list_not_running_fails_a_retry_log(self):
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            self.grade({**self.BY_DESIGN, "dxsamples_hdr": "harness"})
        self.assertEqual(caught.exception.name, "windows_open_rows_not_run")

    def test_a_guest_that_did_not_boot_fails_the_stage(self):
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            self.grade({**self.BY_DESIGN, "dxsamples_hdr": "boot_fail"}, rc=1, kind="FAIL")
        self.assertEqual(caught.exception.name, "windows_open_rows")

    def test_a_retry_or_refusal_without_app_lines_is_a_retry_name(self):
        for line in ("WINAPPS_RETRY action=rerun-on-a-fresh-box class=MEDIA profile=open_source\nWINAPPS_EXIT rc=3 x\n", "WINAPPS_REFUSED why=no-gate-pass\nWINAPPS_EXIT rc=2 x\n"):
            with self.assertRaises(vast_payload.EvidenceError) as caught:
                vast_payload.windows_open_rows_rows(line, 3, SHA, {})
            self.assertEqual(caught.exception.name, "windows_open_rows_retry")

    def test_a_truncated_log_and_a_duplicate_app_are_not_a_pass(self):
        text = self.log(dict(self.BY_DESIGN))
        for bad in (text.replace("WINAPPS_EXIT rc=3 2026-11-17T00:00:00+00:00\n", ""), text + app_line("dxsamples_hdr", "pass")):
            with self.assertRaises(vast_payload.EvidenceError):
                vast_payload.windows_open_rows_rows(bad, 3, SHA, {})

    def test_the_stage_command_is_online_and_names_the_gate_overlay(self):
        source = (HERE / "vast_payload.py").read_text()
        block = source[source.index('if "windows_open_rows" in plan:'):]
        block = block[:block.index('if "propimage_apps" in plan:')]
        for needle in ('"WINVM_NET=online"', '"KF_WINAPPS_PROFILE=" + WINDOWS_OPEN_ROWS_PROFILE', '"KF_WINAPPS_GATE_TAG=" + gate_tag', "windows_apps.sh"):
            self.assertIn(needle, block)

    def test_the_profile_the_stage_runs_exists_and_lists_the_by_design_rows(self):
        gate = json.loads((ROOT / "scripts/bench/windows/appmatrix/gate_profiles.json").read_text())
        block = gate.get("online_source")
        if block is None:
            self.skipTest("online_source needs the open Windows rows (#470)")
        self.assertEqual(list(block["apps"]), list(self.ROWS))
        for name in vast_payload.WINDOWS_OPEN_ROWS_NOT_RUN_BY_DESIGN:
            self.assertIn(name, block["apps"])


class AppsDiscoveryTests(unittest.TestCase):
    def res(self, **verdicts):
        return {k: (v, "-") for k, v in verdicts.items()}

    def test_ran_and_failed_is_a_breakage_and_not_ran_is_named(self):
        g = grade = ads.grade(ads.ROWS, self.res(nvcodec_matrix="PASS", nvml_dcgm="PASS", vkcts="FAIL", vllm="TIMEOUT", tensorrt="NOTRUN", jax="PASS"))
        self.assertEqual(grade["breakages"], ["vkcts", "vllm"])
        self.assertEqual(g["not_run"], ["tensorrt"])
        self.assertEqual(ads.verdict_of(g, strict=True), "FAIL")
        self.assertEqual(ads.verdict_of(g, strict=False), "PARTIAL")

    def test_a_missing_row_is_not_run_never_a_pass(self):
        g = ads.grade(ads.ROWS, self.res(nvcodec_matrix="PASS"))
        self.assertEqual(sorted(g["not_run"]), sorted(set(ads.ROWS) - {"nvcodec_matrix"}))
        self.assertEqual(ads.verdict_of(g), "FAIL")

    def test_every_row_ran_is_a_pass_even_with_breakages(self):
        g = ads.grade(ads.ROWS, self.res(**{r: "FAIL" for r in ads.ROWS}))
        self.assertEqual(g["not_run"], [])
        self.assertEqual(ads.verdict_of(g), "PASS")

    def test_hang_and_a_dead_guest_are_findings_that_ran(self):
        g = ads.grade(ads.ROWS, self.res(**{r: v for r, v in zip(ads.ROWS, ("PASS", "HANG", "GUEST_DEAD", "BOOT_FAIL", "PASS", "PASS"))}))
        self.assertEqual(g["not_run"], [])
        self.assertEqual(g["breakages"], ["nvml_dcgm", "vkcts", "vllm"])

    def test_an_unknown_verdict_word_is_not_a_result(self):
        g = ads.grade(("jax",), self.res(jax="MAYBE"))
        self.assertEqual(g["not_run"], ["jax"])

    def test_the_dcgm_state_of_the_row_is_recorded(self):
        g = ads.grade(("nvml_dcgm",), self.res(nvml_dcgm="PASS"), dcgm_text="NVML_DCGM_SUMMARY nvml_fields_ok=3 dcgm=NOT_RUN dcgm_fields_ok=0/0")
        self.assertEqual(g["rows"][0]["dcgm"], "NOT_RUN")

    def test_read_res_takes_the_later_file_and_the_later_line(self):
        d = tempfile.TemporaryDirectory(prefix="ads-test-")
        self.addCleanup(d.cleanup)
        a, b = Path(d.name) / "guest.res", Path(d.name) / "guest_isolated.res"
        a.write_text("APPRES side=guest app=vkcts verdict=FAIL rc=1 secs=3 quiet=0 note=x\nnoise\nAPPRES side=guest app=jax verdict=PASS rc=0 secs=3 quiet=0 note=-\n")
        b.write_text("APPRES side=guest app=vkcts verdict=PASS rc=0 secs=3 quiet=0 note=-\n")
        self.assertEqual(ads.read_res(a, b)["vkcts"][0], "PASS")
        self.assertEqual(ads.read_res(a)["vkcts"][0], "FAIL")
        self.assertEqual(ads.read_res(Path(d.name) / "absent.res"), {})

    def test_the_plan_names_the_build_and_the_setup_before_the_matrix(self):
        self.assertEqual(ads.plan()["steps"], ["build_vkcts", "setup_mlrows", "overlay", "host", "guest"])
        self.assertEqual(ads.plan(("vllm",))["steps"], ["setup_mlrows", "overlay", "host", "guest"])
        self.assertEqual(ads.plan(("nvcodec_matrix",))["budget_s_per_side"], 3000)

    def test_every_row_of_the_stage_is_a_row_of_run_apps_and_has_a_timeout(self):
        scratch = tempfile.TemporaryDirectory(prefix="ads-list-")
        self.addCleanup(scratch.cleanup)
        import os
        os.environ["TMPDIR"] = scratch.name
        known = ads.known_rows(ROOT)
        for row in ads.ROWS:
            if row in ("nvcodec_matrix", "nvml_dcgm", "vkcts", "vllm", "tensorrt", "jax") and row not in known:
                self.skipTest("the open-row PRs are not all in this tree: " + row)
            self.assertIn(row, ads.ROW_TIMEOUT_S)

    def run_stage(self, results, bundle=True, host=False, runner=None, rows=None, deadline=0, image=True, base=False, setup_log="", setup_rc=0, environ=None,
                  receipt=None, overlay_rc=0, symlink_base=False, boom_on=None):
        """The stage with a fake runner. `image`: the apps image exists in the bench dir; `base`: the base guest image does too; `setup_log`: what `mlrows.py setup`
        printed (the stage reads it from the step's log); `receipt`: the text of guest_apps.qcow2.STAGED; `overlay_rc`: what `qemu-img create` returns;
        `symlink_base`: guest_apps.qcow2 is a link to the base image; `boom_on`: a step whose runner raises. self.last keeps every call the runner saw (argv and env),
        what existed when a boot step started (`boots`) and the bench dir."""
        d = tempfile.TemporaryDirectory(prefix="ads-run-")
        self.addCleanup(d.cleanup)
        root = Path(d.name)
        if bundle:
            (root / "bundle.tgz").write_text("x")
        bench = root / "bench"
        bench.mkdir()
        if image:
            (bench / "guest_apps.qcow2").write_text("apps image")
        if base or symlink_base:
            (bench / "guest.qcow2").write_text("base image")
        if symlink_base:
            (bench / "guest_apps.qcow2").unlink(missing_ok=True)
            (bench / "guest_apps.qcow2").symlink_to(bench / "guest.qcow2")
        if receipt is not None:
            (bench / "guest_apps.qcow2.STAGED").write_text(receipt)
        run_dir = root / "results" / "r1"
        run_dir.mkdir(parents=True)
        calls = []
        self.last = {"argv": [], "env": [], "bench": bench, "boots": []}

        def fake(argv, env, log, timeout):
            calls.append(argv[1:3] if argv[0] == "python3" else argv[2:4])
            self.last["argv"].append(list(argv))
            self.last["env"].append(dict(env))
            if argv[0] == "qemu-img":
                if not overlay_rc:
                    Path(argv[-1]).write_text("overlay")
                return overlay_rc
            if boom_on and boom_on in argv:
                raise RuntimeError("boom")
            if argv[1].endswith("apps_matrix.sh"):
                img = Path(env["KF_GUEST_IMG"])
                sidecar = Path(str(img) + ".STAGED")
                self.last["boots"].append({"side": argv[2], "image": img, "exists": img.exists(), "receipt": sidecar.read_text() if sidecar.exists() else None})
            if "setup" in argv and any(a.endswith("mlrows.py") for a in argv):
                Path(log).write_text(setup_log)
                return setup_rc
            side = "guest" if "guest" in argv else "host" if "host" in argv else None
            if side:
                booted = argv[4:]
                (run_dir / (side + ".res")).write_text("".join("APPRES side=%s app=%s verdict=%s rc=0 secs=1 quiet=0 note=%s\n" % (side, a, v, n)
                                                               for a, (v, n) in results.items() if a in booted))
            return 0

        ns = types.SimpleNamespace(sha=SHA, checkout=str(ROOT), out=str(root / "out.json"), run_id="r1", rows=rows or ",".join(ads.ROWS), apps_root=str(root),
                                   deadline_epoch=deadline, lenient=False)
        hostdir = root / "hostbundle"
        if host:
            hostdir.mkdir()
        env_in = {k: v for k, v in os.environ.items() if k != "KF_GUEST_IMG"}
        env_in["BENCH_DIR"] = str(bench)
        env_in.update({k: v.replace("{bench}", str(bench)) for k, v in (environ or {}).items()})       # "{bench}" is this run's bench dir
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = ads.run(ns, runner=runner or fake, known=set(ads.ROWS), host_bundle=str(hostdir), environ=env_in)
        return rc, json.loads((root / "out.json").read_text()), calls

    ALL_PASS = {r: ("PASS", "-") for r in ads.ROWS}

    def test_a_stage_where_every_row_ran_passes_and_names_the_host_control(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS)
        self.assertEqual((rc, rec["verdict"], rec["host_control"]), (0, "PASS", "absent"))
        rc, rec, calls = self.run_stage(self.ALL_PASS, host=True)
        self.assertEqual((rc, rec["host_control"], rec["rows"][0]["host"]), (0, "present", "PASS"))

    def test_a_row_that_did_not_run_fails_the_stage_by_name(self):
        rc, rec, _ = self.run_stage({**self.ALL_PASS, "tensorrt": ("NOTRUN", "not-run:driver_cuda_below_forced_flavour")})
        self.assertEqual((rc, rec["verdict"], rec["not_run"]), (1, "FAIL", ["tensorrt"]))
        self.assertEqual(rec["reason"], "discovery_not_run:tensorrt")
        self.assertEqual(rec["rows"][4]["reason"], "driver_cuda_below_forced_flavour")

    def test_no_bundle_is_a_retry_and_never_a_pass(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, bundle=False)
        self.assertEqual((rc, rec["verdict"], rec["reason"]), (3, "RETRY", "no_bundle"))
        self.assertEqual(calls, [])

    def test_a_missing_guest_row_fails_the_stage_by_name(self):
        results = {r: v for r, v in self.ALL_PASS.items() if r != "vkcts"}
        rc, rec, _ = self.run_stage(results)
        self.assertEqual((rc, rec["verdict"], rec["not_run"], rec["reason"]), (1, "FAIL", ["vkcts"], "discovery_not_run:vkcts"))

    def test_an_unknown_verdict_word_fails_the_stage_by_name(self):
        rc, rec, _ = self.run_stage({**self.ALL_PASS, "jax": ("MAYBE", "-")})
        self.assertEqual((rc, rec["not_run"]), (1, ["jax"]))

    # ---- 2026-11-20: the first run on a box booted the BASE guest image (the stage never named the apps image), see docs/design/V3_STANDALONE_FULL_RUN.md section 4
    CONFIG_NOTE = "CONFIG cases=291 pass=0 warn=0 ns=0 fail=0 crash=0 timeout=0 notrun=291 gpu=EXTERNAL"
    SETUP_REFUSED = ("MLROWS_FACTS {\"python\":[3,10,12],\"python_h\":false}\n"
                     "MLROWS_SETUP row=vllm status=ok reason=flavour_cu130,_31_s,_9.80_GB\n"
                     "MLROWS_SETUP row=tensorrt status=NOT_RUN reason=python_too_old:_3.10_<_3.12\n"
                     "MLROWS_SETUP row=jax status=NOT_RUN reason=python_too_old:_3.10_<_3.12\n")

    def overlay_of(self):
        return self.last["bench"] / "discovery_r1.qcow2"

    def test_every_step_gets_a_throwaway_overlay_and_never_an_image_itself(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, host=True, base=True)
        self.assertEqual((rc, rec["verdict"]), (0, "PASS"))
        bench = self.last["bench"]
        images = [e.get("KF_GUEST_IMG") for e in self.last["env"]]
        self.assertEqual(len(images), 5)                       # build, setup, overlay, host, guest
        for image in images:
            self.assertEqual(image, str(self.overlay_of()))
            self.assertNotIn(Path(image).resolve(), (( bench / "guest_apps.qcow2").resolve(), (bench / "guest.qcow2").resolve()))
        self.assertEqual({b["side"]: b["exists"] for b in self.last["boots"]}, {"host": True, "guest": True})

    def test_the_overlay_is_backed_by_the_apps_image_the_gate_names(self):
        self.run_stage(self.ALL_PASS, base=True)
        create = next(a for a in self.last["argv"] if a[0] == "qemu-img")
        backing = str((self.last["bench"] / "guest_apps.qcow2").resolve())
        self.assertEqual(backing, str(Path(apps_gate.apps_image({"BENCH_DIR": str(self.last["bench"])})).resolve()))
        self.assertEqual(create, ["qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", backing, str(self.overlay_of())])

    def test_the_overlay_is_removed_when_the_stage_ends(self):
        self.run_stage(self.ALL_PASS, receipt="version=580.159.04\n")
        self.assertEqual([b["image"] for b in self.last["boots"]], [self.overlay_of()])       # it was there while the guest booted
        self.assertFalse(self.overlay_of().exists())
        self.assertFalse(Path(str(self.overlay_of()) + ".STAGED").exists())
        self.assertTrue((self.last["bench"] / "guest_apps.qcow2").exists())            # the image itself is never removed

    def test_the_overlay_is_removed_when_a_step_raises(self):
        with self.assertRaises(RuntimeError):
            self.run_stage(self.ALL_PASS, receipt="version=580.159.04\n", boom_on="guest")
        self.assertFalse(self.overlay_of().exists())
        self.assertFalse(Path(str(self.overlay_of()) + ".STAGED").exists())
        self.assertTrue((self.last["bench"] / "guest_apps.qcow2").exists())

    def test_the_boot_helper_finds_the_driver_receipt_beside_the_overlay(self):
        """boot_nvkvm.sh reads `<image>.STAGED` (guest_profile.sh) and refuses an image without one: the overlay carries the apps image's receipt."""
        self.run_stage(self.ALL_PASS, receipt="version=580.159.04\n")
        self.assertEqual({b["receipt"] for b in self.last["boots"]}, {"version=580.159.04\n"})
        self.run_stage(self.ALL_PASS)
        self.assertEqual({b["receipt"] for b in self.last["boots"]}, {None})

    def test_the_receipt_beside_the_overlay_is_a_copy_of_the_images_own(self):
        self.run_stage(self.ALL_PASS, receipt="version=580.159.04\nsource=staged\n")
        self.assertEqual(len(self.last["boots"]), 1)
        self.assertEqual(self.last["boots"][0]["receipt"], "version=580.159.04\nsource=staged\n")
        self.assertEqual((self.last["bench"] / "guest_apps.qcow2.STAGED").read_text(), "version=580.159.04\nsource=staged\n")      # copied, not moved

    def test_an_overlay_that_could_not_be_made_is_a_retry_and_nothing_boots(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, overlay_rc=1)
        self.assertEqual((rc, rec["verdict"], rec["reason"]), (3, "RETRY", "overlay_failed"))
        self.assertEqual([a for a in self.last["argv"] if a[1].endswith("apps_matrix.sh")], [])

    def test_an_overlay_failure_is_a_retry_in_the_payload_grader(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS, overlay_rc=1)
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            vast_payload.apps_discovery_rows("KF_APPS_DISCOVERY_RESULT " + json.dumps(rec, sort_keys=True, separators=(",", ":")) + "\n", rc, SHA, {})
        self.assertEqual(caught.exception.name, "apps_discovery_retry")

    def test_the_apps_gate_names_the_image_in_one_place(self):
        self.assertEqual(apps_gate.apps_image({"BENCH_DIR": "/b"}), "/b/guest_apps.qcow2")
        self.assertEqual(apps_gate.apps_image({}), "/workspace/bench/guest_apps.qcow2")
        self.assertEqual(apps_gate.apps_image({"BENCH_DIR": "/b", "KF_GUEST_IMG": "/x/img.qcow2"}), "/x/img.qcow2")
        self.assertEqual(apps_gate.base_image({"BENCH_DIR": "/b"}), "/b/guest.qcow2")

    def test_no_apps_image_is_a_retry_before_anything_boots(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, image=False)
        self.assertEqual((rc, rec["verdict"], rec["reason"]), (3, "RETRY", "no_apps_image"))
        self.assertEqual((calls, self.last["argv"]), ([], []))

    def test_the_base_guest_alone_is_not_an_apps_image(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, image=False, base=True)
        self.assertEqual((rc, rec["reason"], calls), (3, "no_apps_image", []))
        self.assertEqual([e for e in self.last["env"] if "guest.qcow2" in e.get("KF_GUEST_IMG", "")], [])

    def test_a_kf_guest_img_that_names_the_base_image_is_refused_by_name_before_any_boot(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, base=True, environ={"KF_GUEST_IMG": "{bench}/guest.qcow2"})
        self.assertEqual((rc, rec["verdict"], rec["reason"]), (1, "FAIL", "not_apps_image"))
        self.assertEqual((calls, self.last["argv"]), ([], []))

    def test_any_other_image_in_kf_guest_img_is_refused_too(self):
        other_dir = tempfile.TemporaryDirectory(prefix="ads-img-")
        self.addCleanup(other_dir.cleanup)
        other = Path(other_dir.name) / "guest.qcow2"                 # even a file that is called guest.qcow2 elsewhere: it is not THE apps image
        other.write_text("x")
        rc, rec, calls = self.run_stage(self.ALL_PASS, environ={"KF_GUEST_IMG": str(other)})
        self.assertEqual((rc, rec["verdict"], rec["reason"], calls), (1, "FAIL", "not_apps_image", []))

    def test_a_kf_guest_img_that_is_the_apps_image_itself_is_accepted(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, environ={"KF_GUEST_IMG": "{bench}/guest_apps.qcow2"})
        self.assertEqual((rc, rec["verdict"]), (0, "PASS"))

    def test_an_apps_image_that_is_a_link_to_the_base_image_is_refused(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, symlink_base=True)
        self.assertEqual((rc, rec["verdict"], rec["reason"], calls), (1, "FAIL", "not_apps_image", []))

    def test_the_link_is_refused_even_when_kf_guest_img_names_the_link_itself(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, symlink_base=True, environ={"KF_GUEST_IMG": "{bench}/guest_apps.qcow2"})
        self.assertEqual((rc, rec["reason"], calls), (1, "not_apps_image", []))

    def test_the_refusal_is_a_failure_in_the_payload_grader(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS, base=True, environ={"KF_GUEST_IMG": "{bench}/guest.qcow2"})
        vast_report.validate_apps_discovery(rec)
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            vast_payload.apps_discovery_rows("KF_APPS_DISCOVERY_RESULT " + json.dumps(rec, sort_keys=True, separators=(",", ":")) + "\n", rc, SHA, {})
        self.assertEqual(caught.exception.name, "apps_discovery")

    def test_the_missing_image_is_a_retry_in_the_payload_grader_too(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS, image=False)
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            vast_payload.apps_discovery_rows("KF_APPS_DISCOVERY_RESULT " + json.dumps(rec, sort_keys=True, separators=(",", ":")) + "\n", rc, SHA, {})
        self.assertEqual(caught.exception.name, "apps_discovery_retry")

    def test_a_config_row_is_not_run_and_never_a_breakage(self):
        g = ads.grade(ads.ROWS, {**self.res(**{r: "PASS" for r in ads.ROWS}), "vkcts": ("FAIL", self.CONFIG_NOTE)})
        self.assertEqual((g["not_run"], g["breakages"]), (["vkcts"], []))
        entry = next(r for r in g["rows"] if r["name"] == "vkcts")
        self.assertEqual((entry["status"], entry["guest"]), ("NOT_RUN", "FAIL"))
        self.assertTrue(entry["reason"].startswith("CONFIG"), entry)
        self.assertEqual(ads.verdict_of(g), "FAIL")
        plain = ads.grade(("vkcts",), {"vkcts": ("FAIL", "VKCTS_SUMMARY fail=3")})
        self.assertEqual((plain["not_run"], plain["breakages"]), ([], ["vkcts"]))

    def test_a_config_row_fails_the_stage_by_name(self):
        rc, rec, _ = self.run_stage({**self.ALL_PASS, "vkcts": ("FAIL", self.CONFIG_NOTE)})
        self.assertEqual((rc, rec["verdict"], rec["not_run"], rec["breakages"], rec["reason"]), (1, "FAIL", ["vkcts"], [], "discovery_not_run:vkcts"))

    def test_a_hang_without_a_result_line_is_not_run_and_a_dead_guest_is_not(self):
        note = "no-result-line(guest_alive=1)"
        g = ads.grade(("nvcodec_matrix", "nvml_dcgm"), {"nvcodec_matrix": ("HANG", note), "nvml_dcgm": ("GUEST_DEAD", "no-result-line(guest_alive=0)")})
        self.assertEqual((g["not_run"], g["breakages"]), (["nvcodec_matrix"], ["nvml_dcgm"]))
        self.assertTrue(g["rows"][0]["reason"].startswith("hang_"), g["rows"][0])

    def test_a_dead_guest_is_a_listed_breakage_in_the_stage(self):
        rc, rec, _ = self.run_stage({**self.ALL_PASS, "jax": ("GUEST_DEAD", "no-result-line(guest_alive=0)")})
        self.assertEqual((rc, rec["verdict"], rec["not_run"], rec["breakages"]), (0, "PASS", [], ["jax"]))

    def test_a_hang_without_a_result_line_fails_the_stage_by_name(self):
        rc, rec, _ = self.run_stage({**self.ALL_PASS, "nvcodec_matrix": ("HANG", "no-result-line(guest_alive=1)")})
        self.assertEqual((rc, rec["not_run"], rec["breakages"]), (1, ["nvcodec_matrix"], []))

    def test_setup_refusals_are_read_with_their_real_reason(self):
        got = ads.setup_refusals(self.SETUP_REFUSED, ads.ML_ROWS)
        self.assertEqual(got, {"tensorrt": "python_too_old:3.10<3.12", "jax": "python_too_old:3.10<3.12"})
        self.assertEqual(ads.setup_refusals(self.SETUP_REFUSED, ("vllm",)), {})          # only the rows the stage asked for
        self.assertEqual(ads.setup_refusals("", ads.ML_ROWS), {})

    def test_only_a_refusal_is_a_refusal(self):
        text = ("MLROWS_SETUP row=vllm status=FAIL reason=pip_failed:_boom\n"
                "MLROWS_SETUP row=jax status=ok reason=already_set_up\n"
                "MLROWS_SETUP row=nvml_dcgm status=NOT_RUN reason=not_an_ml_row\n"
                "noise MLROWS_SETUP row=tensorrt status=NOT_RUN reason=late\n")
        self.assertEqual(ads.setup_refusals(text, ads.ML_ROWS), {})

    def test_a_refusal_with_a_long_or_odd_reason_is_bounded_and_printable(self):
        got = ads.setup_refusals("MLROWS_SETUP row=jax status=NOT_RUN reason=" + "x\u00e9;`$(id)" * 40 + "\n", ads.ML_ROWS)
        self.assertRegex(got["jax"], r"^[A-Za-z0-9_.:=<>-]{1,80}$")

    def test_rows_the_host_setup_refused_are_not_booted_and_say_why(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, setup_log=self.SETUP_REFUSED)
        self.assertEqual((rc, rec["verdict"], rec["not_run"], rec["reason"]), (1, "FAIL", ["tensorrt", "jax"], "discovery_not_run:tensorrt,jax"))
        by_name = {r["name"]: r for r in rec["rows"]}
        for name in ("tensorrt", "jax"):
            self.assertEqual((by_name[name]["status"], by_name[name]["guest"], by_name[name]["reason"]), ("NOT_RUN", "-", "python_too_old:3.10<3.12"))
        for name in ("nvcodec_matrix", "nvml_dcgm", "vkcts", "vllm"):
            self.assertEqual(by_name[name]["status"], "PASS")
        guest = next(a for a in self.last["argv"] if "guest" in a)
        self.assertEqual(guest[4:], ["nvcodec_matrix", "nvml_dcgm", "vkcts", "vllm"])
        self.assertEqual(sum("guest" in a for a in self.last["argv"]), 1)

    def test_a_refused_row_is_not_booted_on_the_host_control_either(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, host=True, setup_log=self.SETUP_REFUSED)
        for side in ("host", "guest"):
            argv = next(a for a in self.last["argv"] if side in a and a[1].endswith("apps_matrix.sh"))
            self.assertEqual(argv[4:], ["nvcodec_matrix", "nvml_dcgm", "vkcts", "vllm"], side)

    def test_the_refusal_is_in_the_record_the_payload_reads(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS, setup_log=self.SETUP_REFUSED)
        vast_report.validate_apps_discovery(rec)
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            vast_payload.apps_discovery_rows("KF_APPS_DISCOVERY_RESULT " + json.dumps(rec, sort_keys=True, separators=(",", ":")) + "\n", rc, SHA, {})
        self.assertEqual(caught.exception.name, "apps_discovery_not_run")

    def test_when_every_row_is_refused_nothing_boots_and_the_stage_fails_by_name(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, rows="tensorrt,jax", setup_log=self.SETUP_REFUSED)
        self.assertEqual((rc, rec["verdict"], rec["reason"], rec["not_run"]), (1, "FAIL", "discovery_not_run:tensorrt,jax", ["tensorrt", "jax"]))
        self.assertEqual([s["step"] for s in rec["steps"]], ["setup_mlrows"])
        self.assertEqual([a for a in self.last["argv"] if a[1].endswith("apps_matrix.sh")], [])

    def test_all_refused_is_a_failure_of_the_record_and_never_a_retry(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS, rows="tensorrt,jax", setup_log=self.SETUP_REFUSED)
        self.assertNotEqual(rec["verdict"], "RETRY")
        vast_report.validate_apps_discovery(rec)

    def test_a_setup_that_failed_is_not_a_refusal_the_guest_names_it(self):
        log = "MLROWS_SETUP row=vllm status=FAIL reason=pip_failed:_boom\n"
        rc, rec, _ = self.run_stage({**self.ALL_PASS, "vllm": ("NOTRUN", "not-run:not_set_up")}, setup_rc=3, setup_log=log)
        guest = next(a for a in self.last["argv"] if "guest" in a)
        self.assertIn("vllm", guest[4:])
        self.assertEqual((rc, rec["not_run"]), (1, ["vllm"]))

    def test_the_budget_is_the_sum_of_the_rows_timeouts_per_side(self):
        import time
        budget = ads.plan(("jax",))["budget_s_per_side"]
        rc, rec, calls = self.run_stage({"jax": ("PASS", "-")}, rows="jax", deadline=int(time.time()) + budget - 5)
        self.assertEqual((rc, rec["reason"]), (3, "no_time_left"))
        rc, rec, calls = self.run_stage({"jax": ("PASS", "-")}, rows="jax", deadline=int(time.time()) + budget + 600)
        self.assertEqual((rc, rec["verdict"]), (0, "PASS"))

    def test_no_bundle_is_printed_as_the_record_line_and_written(self):
        d = tempfile.TemporaryDirectory(prefix="ads-nb-")
        self.addCleanup(d.cleanup)
        ns = types.SimpleNamespace(sha=SHA, checkout=str(ROOT), out=str(Path(d.name) / "o.json"), run_id="r1", rows="jax", apps_root=d.name, deadline_epoch=0, lenient=False)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = ads.run(ns, runner=lambda *a: 0, known={"jax"}, host_bundle=d.name)
        self.assertEqual(rc, 3)
        self.assertIn('KF_APPS_DISCOVERY_RESULT {', out.getvalue())
        self.assertIn('"verdict":"RETRY"', out.getvalue())
        self.assertEqual(json.loads((Path(d.name) / "o.json").read_text())["reason"], "no_bundle")

    def test_not_enough_time_is_a_retry_before_anything_starts(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, deadline=1)
        self.assertEqual((rc, rec["reason"], calls), (3, "no_time_left", []))

    def test_a_guest_that_wrote_no_result_is_a_retry(self):
        rc, rec, _ = self.run_stage({})
        self.assertEqual((rc, rec["reason"]), (3, "no_guest_results"))

    def test_a_failed_vkcts_build_is_a_retry_not_a_verdict_on_the_guest(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, runner=lambda argv, env, log, timeout: 1 if "build" in argv else 0)
        self.assertEqual((rc, rec["reason"]), (3, "vkcts_build_failed"))

    def test_the_steps_run_in_order_and_the_host_runs_first_when_it_exists(self):
        rc, rec, calls = self.run_stage(self.ALL_PASS, host=True)
        steps = [s["step"] for s in rec["steps"]]
        self.assertEqual(steps, ["build_vkcts", "setup_mlrows", "overlay", "host", "guest"])

    def test_a_pass_record_that_lists_a_not_run_row_is_refused_by_name(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS)
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            vast_payload.apps_discovery_rows("KF_APPS_DISCOVERY_RESULT " + json.dumps({**rec, "not_run": ["jax"]}) + "\n", 0, SHA, {})
        self.assertEqual(caught.exception.name, "apps_discovery_not_run")

    def test_a_retry_record_is_the_stage_failing_by_a_retry_name(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS)
        with self.assertRaises(vast_payload.EvidenceError) as caught:
            vast_payload.apps_discovery_rows("KF_APPS_DISCOVERY_RESULT " + json.dumps({**rec, "verdict": "RETRY", "reason": "no_bundle", "not_run": []}) + "\n", 3, SHA, {})
        self.assertEqual(caught.exception.name, "apps_discovery_retry")

    def test_the_payload_grader_maps_the_record(self):
        rc, rec, _ = self.run_stage(self.ALL_PASS)
        text = "KF_APPS_DISCOVERY_RESULT " + json.dumps(rec, sort_keys=True, separators=(",", ":")) + "\n"
        result = {}
        rows = vast_payload.apps_discovery_rows(text, 0, SHA, result)
        self.assertEqual((rows[0]["name"], rows[0]["pass"]), ("apps_discovery", 6))
        vast_report.validate_apps_discovery(result["apps_discovery"])
        for change, name in (({"verdict": "RETRY"}, "apps_discovery_retry"), ({"verdict": "FAIL", "not_run": ["jax"]}, "apps_discovery_not_run"), ({"sha": "0" * 40}, "apps_discovery")):
            with self.assertRaises(vast_payload.EvidenceError) as caught:
                vast_payload.apps_discovery_rows("KF_APPS_DISCOVERY_RESULT " + json.dumps({**rec, **change}) + "\n", 0, SHA, {})
            self.assertEqual(caught.exception.name, name)
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.apps_discovery_rows(text, 1, SHA, {})
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.apps_discovery_rows("", 0, SHA, {})


class ReportValidatorTests(unittest.TestCase):
    def doc(self, **extra):
        base = {"verdict": "PASS", "suites": [{"name": "provision", "pass": 1, "fail": 0, "skip": 0}]}
        base.update(extra)
        return base

    def read(self, doc):
        d = tempfile.TemporaryDirectory(prefix="report-test-")
        self.addCleanup(d.cleanup)
        path = Path(d.name) / "r.json"
        path.write_text(json.dumps(doc))
        return vast_report.read_result_file(str(path))

    def test_full_record_is_accepted_and_bad_ones_are_not(self):
        good = sf.record(list(sf.FULL_STAGES), ())
        self.read(self.doc(full=good))
        for bad in ({**good, "complete": True, "skipped": ["matlab"]}, {**good, "manifest": 0}, {**good, "stages": ["Bad Name"]}, {k: v for k, v in good.items() if k != "stages"}):
            with self.assertRaises(Exception):
                self.read(self.doc(full=bad))

    def test_a_run_that_skipped_a_stage_cannot_claim_to_be_complete(self):
        bad = {**sf.record([s for s in sf.FULL_STAGES if s != "matlab"], ("matlab",)), "complete": True}
        with self.assertRaises(Exception):
            self.read(self.doc(full=bad))

    def test_the_reader_takes_twenty_five_rows_one_full_run_reports(self):
        rows = [{"name": "stage_%d" % i, "pass": 1, "fail": 0, "skip": 0} for i in range(25)]
        self.assertEqual(len(self.read({"verdict": "PASS", "suites": rows})["suites"]), 25)

    def test_open_rows_cannot_have_run_more_than_expected(self):
        good = {"profile": "open_source", "expected": 9, "ran": 9, "passed": 9, "breakages": [], "not_run_by_design": [], "not_run": [], "sha": SHA}
        self.read(self.doc(windows_open_rows=good))
        with self.assertRaises(Exception):
            self.read(self.doc(windows_open_rows={**good, "passed": 10, "ran": 10, "expected": 9}))

    def test_windows_open_rows_record(self):
        good = {"profile": "open_source", "expected": 9, "ran": 6, "passed": 5, "breakages": ["dxsamples_hdr"], "not_run_by_design": ["vkglcts_gl46", "vkglcts_vk", "dxsamples_multiadapter"],
                "not_run": [], "sha": SHA}
        self.read(self.doc(windows_open_rows=good))
        with self.assertRaises(Exception):
            self.read(self.doc(windows_open_rows={**good, "ran": 10}))
        with self.assertRaises(Exception):
            self.read(self.doc(windows_open_rows={**good, "not_run": ["dxsamples_hdr"]}))

    def test_a_full_run_of_rows_fits_and_one_more_than_the_bound_does_not(self):
        rows = [{"name": "stage_%d" % i, "pass": 1, "fail": 0, "skip": 0} for i in range(vast_report.MAX_SUITE_ROWS)]
        self.read({"verdict": "PASS", "suites": rows})
        with self.assertRaises(Exception):
            self.read({"verdict": "PASS", "suites": rows + [{"name": "one_more", "pass": 1, "fail": 0, "skip": 0}]})


if __name__ == "__main__":
    unittest.main()
