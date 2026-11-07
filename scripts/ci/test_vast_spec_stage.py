#!/usr/bin/env python3
"""The SPECviewperf gate stage (scripts/bench/box/specviewperf_stage.sh and its wrapper), GPU-free.

FALSIFIER (written before the code): the stage prints a PASS while a viewset failed, was refused, hung, reset the
guest or never ran; treats missing or wrong media, a transport fault, a boot that never came or the time running
out as anything but RETRY (never a pass); starts a viewset it has no time for; skips the settle wait after a boot;
leaves the downloaded copy or the overlay that holds the tree behind; lets a score, a note or any non-status line
reach the streamed log; trusts a copy the guest does not confirm; or passes on a red host reference.

The REAL stage script runs against fakes of everything that needs a box (spec_stage_fakes.py): a fake win_vm.sh over
a scenario directory, a fake fetch script, a clock that moves only when the stage sleeps. Every rule has at least
two tests: spec_stage_mutants.py weakens each rule once and requires two of them to fail.
"""

import atexit
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

import spec_stage_fakes as fk

LANE_A = ["energy-04", "creo-04", "blender-01", "enscape-01"]
LANE_B = ["medical-04", "maya-07", "catia-07", "solidworks-08", "unreal_engine-01"]
PUBLIC = re.compile(
    r"^(?:SPEC_STAGE_LIST lane=(?:lanea|laneb|both) n=[0-9]{1,2}"
    r"|SPEC_ROW name=[a-z][a-z_]*-[0-9]{2} result=(?:PASS|FAIL|NOT_RUN) subtests=[0-9]{1,3}/[0-9]{1,3} reason=(?:[A-Z_]{1,16}|-) failed=(?:[A-Za-z0-9_,]{1,120}|-)"
    r"|[a-z][a-z_]*_[0-9]{2} --- FAILED"
    r"|SPEC_STAGE_COUNTS pass=[0-9]+ fail=[0-9]+ not_run=[0-9]+ of=[0-9]+"
    r"|SPEC_COPY_DELETED removed=(?:archive|overlay|archive,overlay|nothing)"
    r"|SPEC_STAGE_START lane=(?:lanea|laneb|both|none)"
    r"|SPEC_NOTE .*"
    r"|SPECVIEWPERF_STAGE_(?:PASS|FAIL|RETRY class=[A-Z0-9_]+)|SPECVIEWPERF_STAGE_EXIT rc=[0-9]+)$"
)


# One run of the stage per distinct scenario: the tests of one scenario read the same finished sandbox (a run costs seconds,
# and a scenario is only read after it ends). A test that prepares the sandbox itself passes cache=False.
_CACHE = {}


@atexit.register
def _drop_cached_sandboxes():
    for sb, _ in _CACHE.values():
        shutil.rmtree(sb.root, ignore_errors=True)


class StageCase(unittest.TestCase):
    def setUp(self):
        self._owned = True
        self._dir = tempfile.mkdtemp(prefix="kfspec-")
        self.addCleanup(lambda: shutil.rmtree(self._dir, ignore_errors=True))
        self.sb = fk.Sandbox(self._dir)

    def go(self, lane="lanea", budget=40000, cache=True, **scn):
        key = (lane, budget, tuple(sorted(scn.items())))
        if cache and key in _CACHE:
            self.sb, self.p = _CACHE[key]
        else:
            if cache:
                # a cached sandbox outlives this test: it has a directory of its own that atexit removes
                self.sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-keep-"))
            for k, v in scn.items():
                self.sb.set(k.replace("__", "."), v)
            self.p = self.sb.run(lane, budget)
            if cache:
                _CACHE[key] = (self.sb, self.p)
        self.rows, self.order = fk.rows_of(self.p.stdout)
        self.verdict = fk.verdict_of(self.p.stdout)
        return self.p

    def results(self):
        return {n: (r["result"], r["reason"]) for n, r in self.rows.items()}

    def stage_log(self):
        return self.sb.read("prov/%s_spec/stage.log" % self.sb.tag)

    def archive_gone(self):
        return not (self.sb.root / "store/specviewperf-15.0.0.tar.zst").exists()

    def overlay_gone(self):
        return not (self.sb.vm / "disk/spec_ci0123abcd.qcow2").exists()

    def guest_stopped(self):
        pid = self.sb.vm / "run/qemu.pid"
        if not pid.exists():
            return True
        return not Path("/proc/%s" % pid.read_text().strip()).exists() or "Z" in Path(
            "/proc/%s/stat" % pid.read_text().strip()
        ).read_text().split(")")[-1][:3]


class HappyPath(StageCase):
    def test_lane_a_passes_in_order_with_one_row_per_viewset(self):
        self.go("lanea")
        self.assertEqual(self.p.returncode, 0, self.p.stdout)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_PASS")
        self.assertEqual(self.order, LANE_A)
        self.assertEqual({r["result"] for r in self.rows.values()}, {"PASS"})
        self.assertIn("SPEC_STAGE_COUNTS pass=4 fail=0 not_run=0 of=4", self.p.stdout)
        self.assertIn("SPEC_STAGE_LIST lane=lanea n=4", self.p.stdout)

    def test_lane_b_passes_in_order_with_one_row_per_viewset(self):
        self.go("laneb")
        self.assertEqual(self.p.returncode, 0, self.p.stdout)
        self.assertEqual(self.order, LANE_B)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_PASS")

    def test_both_runs_the_nine_viewsets_lane_a_then_lane_b(self):
        self.go("both")
        self.assertEqual(self.order, LANE_A + LANE_B)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_PASS")

    def test_every_line_the_stage_prints_is_a_status_line_or_a_note(self):
        self.go("lanea")
        for line in self.p.stdout.splitlines():
            self.assertRegex(line, PUBLIC)

    def test_the_stage_prints_one_verdict_line_last(self):
        self.go("lanea")
        verdicts = [l for l in self.p.stdout.splitlines() if l.startswith("SPECVIEWPERF_STAGE_")]
        self.assertEqual(len(verdicts), 1)
        self.assertEqual(self.p.stdout.splitlines()[-1], verdicts[0])

    def test_the_viewsets_run_in_ascending_native_time(self):
        import json

        t = json.loads((fk.BENCH / "specviewperf/lanes.json").read_text())
        for lane in ("lanea", "laneb"):
            walls = [t["viewsets"][n]["native_s"] for n in t["lanes"][lane]]
            self.assertEqual(walls, sorted(walls), lane)


class Inputs(StageCase):
    """Missing or wrong media is RETRY class MEDIA; the pin is the repo's, never the Worker's."""

    def test_the_fetch_asks_for_the_object_with_the_pin_and_a_store_path(self):
        self.go("lanea")
        line = self.sb.read("fetch.log").splitlines()[0]
        self.assertIn("specviewperf-15.0.0.tar.zst", line)
        self.assertIn("--sha256 " + self.sb.sha, line)
        self.assertIn("--out " + str(self.sb.root / "store/specviewperf-15.0.0.tar.zst"), line)
        self.assertEqual(len(self.sb.read("fetch.log").splitlines()), 1)

    def test_a_missing_pin_is_retry_media_and_nothing_is_fetched_or_booted(self):
        sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"), pin=False)
        p = sb.run("lanea")
        self.assertEqual(p.returncode, 3, p.stdout)
        self.assertEqual(fk.verdict_of(p.stdout), "SPECVIEWPERF_STAGE_RETRY class=MEDIA")
        self.assertEqual(sb.read("fetch.log"), "")
        self.assertEqual(sb.runs(), [])

    def test_a_pin_line_of_another_object_is_not_a_pin(self):
        sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"), pin=False)
        (sb.src / "scripts/bench/box/specviewperf.pin").write_text("specviewperf-15.0.0.tar sha256=%s\n" % ("a" * 64))
        p = sb.run("lanea")
        self.assertEqual(fk.verdict_of(p.stdout), "SPECVIEWPERF_STAGE_RETRY class=MEDIA")
        self.assertEqual(sb.read("fetch.log"), "")

    def test_a_sha256_mismatch_is_retry_media_and_no_viewset_runs(self):
        self.go("lanea", fetch="sha")
        self.assertEqual(self.p.returncode, 3)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=MEDIA")
        self.assertEqual({r["result"] for r in self.rows.values()}, {"NOT_RUN"})
        self.assertNotIn("launch ", self.sb.calls())

    def test_an_object_the_worker_does_not_know_is_retry_media(self):
        self.go("lanea", fetch="notfound")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=MEDIA")
        self.assertEqual(self.p.returncode, 3)

    def test_a_network_failure_of_the_download_is_retry_provider(self):
        self.go("lanea", fetch="net")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=PROVIDER")

    def test_a_download_that_ran_out_of_time_is_retry_time(self):
        self.go("lanea", fetch="timeout")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TIME")

    def test_a_download_failing_with_an_unknown_status_is_retry_provider(self):
        self.go("lanea", fetch="other")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=PROVIDER")

    def test_a_download_out_of_time_leaves_the_lane_not_run_and_launches_nothing(self):
        self.go("lanea", fetch="timeout")
        self.assertEqual({v for v in self.results().values()}, {("NOT_RUN", "SETUP")})
        self.assertNotIn("launch ", self.sb.calls())
        self.assertEqual(self.p.returncode, 3)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TIME")

    def test_a_fetch_helper_refusing_its_environment_is_retry_operator(self):
        self.go("lanea", fetch="usage")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=OPERATOR")

    def test_every_setup_failure_marks_the_lane_not_run_never_passed(self):
        self.go("laneb", fetch="sha")
        self.assertEqual(self.order, LANE_B)
        self.assertEqual({v for v in self.results().values()}, {("NOT_RUN", "SETUP")})
        self.assertNotIn("SPECVIEWPERF_STAGE_PASS", self.p.stdout)


class Preflight(StageCase):
    def test_no_gate_pass_is_retry_operator_and_no_guest_starts(self):
        sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"), gate_pass=False)
        p = sb.run("lanea")
        self.assertEqual((p.returncode, fk.verdict_of(p.stdout)), (3, "SPECVIEWPERF_STAGE_RETRY class=OPERATOR"))
        self.assertEqual(sb.runs(), [])

    def test_a_gate_that_only_retried_is_not_a_gate_pass(self):
        sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"))
        (sb.prov / (sb.tag + "_windows.log")).write_text("WINDOWS_GATE_RETRY class=HOSTREF\nWINDOWS_GATE_EXIT rc=3 now\n")
        p = sb.run("lanea")
        self.assertEqual(fk.verdict_of(p.stdout), "SPECVIEWPERF_STAGE_RETRY class=OPERATOR")
        self.assertEqual(sb.runs(), [])

    def test_too_little_disk_is_retry_provider(self):
        self.go("lanea", disk="100")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=PROVIDER")
        self.assertEqual(self.sb.runs(), [])
        self.assertEqual(self.sb.read("fetch.log"), "")

    def test_the_disk_need_of_both_lanes_is_larger_than_of_one(self):
        self.go("both", disk="150")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=PROVIDER")
        self.go("lanea", disk="150")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_PASS")

    def test_too_little_ram_is_retry_provider(self):
        self.go("lanea", ram="20")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=PROVIDER")
        self.assertEqual(self.sb.runs(), [])

    def test_the_ram_need_is_24_gb(self):
        self.go("lanea", ram="23")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=PROVIDER")
        self.go("lanea", ram="24")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_PASS")

    def test_usage_errors_are_refused_before_anything_runs(self):
        for lane, budget in (("lanec", 100), ("", 100), ("both", None)):
            sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"))
            env = sb.env(lane or "x", budget)
            if not lane:
                env.pop("KF_SPEC_LANE")
            if budget is None:
                env.pop("KF_SPEC_BUDGET_S")
            p = subprocess.run(sb.command(), env=env, capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
            self.assertEqual(p.returncode, 2, (lane, budget, p.stdout))
            self.assertEqual(p.stdout.strip(), "SPECVIEWPERF_STAGE_RETRY class=USAGE")
            self.assertEqual(sb.runs(), [])

    def test_a_bad_tag_is_refused(self):
        p = subprocess.run(["bash", str(self.sb.src / "scripts/bench/box/specviewperf_stage.sh"), "../x", "/tmp"],
                           env=self.sb.env(), capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
        self.assertEqual((p.returncode, p.stdout.strip()), (2, "SPECVIEWPERF_STAGE_RETRY class=USAGE"))

    def test_a_red_host_reference_before_the_run_is_retry_hostref_and_no_guest_starts(self):
        # cup8 fails from its first call when the scenario says so and the first call is the 'before' one
        self.sb.set("cup8_fail_after")
        (self.sb.root / "cup8_calls").write_text("1")
        self.go("lanea", cache=False)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=HOSTREF")
        self.assertEqual(self.sb.runs(), [])
        self.assertEqual({v for v in self.results().values()}, {("NOT_RUN", "SETUP")})

    def test_a_red_host_reference_before_deletes_the_copy_and_leaves_no_overlay(self):
        self.sb.set("cup8_fail_after")
        (self.sb.root / "cup8_calls").write_text("1")
        self.go("lanea", cache=False)
        self.assertTrue(self.archive_gone())
        self.assertTrue(self.overlay_gone())
        self.assertNotIn("SPECVIEWPERF_STAGE_PASS", self.p.stdout)
        self.assertEqual(self.sb.runs(), [])
        self.assertNotIn("launch ", self.sb.calls())

    def test_the_stage_does_not_start_with_a_deadline_already_past(self):
        p = subprocess.run(self.sb.command(), env=self.sb.env(budget=None, KF_SPEC_DEADLINE_EPOCH=int(time.time()) - 5),
                           capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
        self.assertEqual((p.returncode, p.stdout.strip()), (3, "SPECVIEWPERF_STAGE_RETRY class=TIME"))
        self.assertEqual(self.sb.runs(), [])


class Copy(StageCase):
    def test_the_copy_boot_has_no_kf3_and_the_run_boot_has_it(self):
        self.go("lanea")
        runs = self.sb.runs()
        self.assertEqual(len(runs), 2)
        self.assertIn("nokf3=1", runs[0])
        self.assertIn("--no-kf3", runs[0])
        self.assertIn("nokf3=0", runs[1])
        self.assertIn("--guest-driver 580.65.06", runs[1])
        self.assertIn("--kf3-extra gpu-uuid=GPU-" + fk.VM_UUID, runs[1])
        self.assertIn("--env-file", runs[1])

    def test_both_boots_run_the_child_overlay_never_the_gate_overlay(self):
        self.go("lanea")
        for line in self.sb.runs():
            self.assertIn("overlay=spec_ci0123abcd", line)
            self.assertNotIn("overlay=gate_", line)
        self.assertIn("qemu-img create backing=%s" % (self.sb.vm / "disk/gate_ci0123abcd.qcow2"), self.sb.calls())

    def test_the_gate_overlay_is_sealed_read_only(self):
        self.go("lanea")
        mode = (self.sb.vm / "disk/gate_ci0123abcd.qcow2").stat().st_mode & 0o777
        self.assertEqual(mode, 0o400)

    def test_the_guest_gets_16_gb_of_ram(self):
        self.go("lanea")
        for line in self.sb.runs():
            self.assertIn("ram=16384", line)

    def test_the_copy_boot_gets_the_ram_too(self):
        self.go("lanea")
        self.assertIn("ram=16384", self.sb.runs()[0])

    def test_the_run_boot_gets_the_ram_too(self):
        self.go("lanea")
        self.assertIn("ram=16384", self.sb.runs()[1])

    def test_the_run_boot_carries_the_gate_flags_file(self):
        self.go("lanea")
        self.assertIn("--env-file " + str(self.sb.root / "flags.env"), self.sb.runs()[1])

    def test_the_copy_boot_is_started_without_kf3_and_without_the_gate_flags(self):
        self.go("lanea")
        self.assertIn("--no-kf3", self.sb.runs()[0])
        self.assertNotIn("--env-file", self.sb.runs()[0])
        self.assertNotIn("--guest-driver", self.sb.runs()[0])

    def test_an_archive_with_an_absolute_member_is_refused_as_media(self):
        sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"), bad="absolute")
        p = sb.run("lanea")
        self.assertEqual(fk.verdict_of(p.stdout), "SPECVIEWPERF_STAGE_RETRY class=MEDIA")
        self.assertNotIn("launch ", sb.calls())

    def test_the_gate_overlay_stays_sealed_after_a_failed_run_too(self):
        self.go("lanea", **{"viewset__creo-04": "fail_subtest"})
        self.assertEqual((self.sb.vm / "disk/gate_ci0123abcd.qcow2").stat().st_mode & 0o777, 0o400)
        self.assertEqual((self.sb.vm / "disk/gate_ci0123abcd.qcow2").read_text(), "fake gate overlay\n")

    def test_only_the_lanes_viewsets_are_streamed_into_the_guest(self):
        self.go("laneb")
        import tarfile

        with tarfile.open(self.sb.root / "guest_stream.tar") as t:
            dirs = {part for m in t for part in m.name.split("/") if re.match(r"^[a-z_]+-[0-9]{2}-[0-9.]+$", part)}
        self.assertEqual({d.rsplit("-", 1)[0] for d in dirs}, set(LANE_B))
        for other in LANE_A:
            self.assertFalse(any(d.startswith(other + "-") for d in dirs), other)

    def test_both_lanes_stream_the_whole_tree(self):
        self.go("both")
        import tarfile

        with tarfile.open(self.sb.root / "guest_stream.tar") as t:
            dirs = {part.rsplit("-", 1)[0] for m in t for part in m.name.split("/") if re.match(r"^[a-z_]+-[0-9]{2}-[0-9.]+$", part)}
        self.assertEqual(dirs, set(LANE_A + LANE_B))

    def test_a_guest_that_holds_another_tree_than_the_one_streamed_is_retry_transport(self):
        self.go("lanea", verify="corrupt")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TRANSPORT")
        self.assertNotIn("launch ", self.sb.calls())

    def test_a_guest_that_lists_nothing_is_retry_transport(self):
        self.go("lanea", verify="empty")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TRANSPORT")

    def test_a_failed_tar_in_the_guest_is_retry_transport(self):
        self.go("lanea", tar_rc="3")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TRANSPORT")
        self.assertEqual(self.p.returncode, 3)

    def test_a_scp_failure_is_retry_transport(self):
        self.go("lanea", scp_fail="1")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TRANSPORT")

    def test_a_tree_without_the_cli_is_retry_media(self):
        self.go("lanea", cli_absent="1")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=MEDIA")
        self.assertNotIn("launch ", self.sb.calls())

    def test_a_copy_boot_that_never_answers_is_retry_transport_not_a_guest_failure(self):
        self.go("lanea", **{"boot_fail__copy1": "nossh"})
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TRANSPORT")
        self.assertNotIn("FAIL", {r["result"] for r in self.rows.values()})

    def test_an_archive_with_a_link_member_is_refused_as_media(self):
        sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"), bad="symlink")
        p = sb.run("lanea")
        self.assertEqual(fk.verdict_of(p.stdout), "SPECVIEWPERF_STAGE_RETRY class=MEDIA")
        self.assertNotIn("launch ", sb.calls())

    def test_an_archive_with_a_dotdot_member_is_refused_as_media(self):
        sb = fk.Sandbox(tempfile.mkdtemp(prefix="kfspec-"), bad="dotdot")
        p = sb.run("lanea")
        self.assertEqual(fk.verdict_of(p.stdout), "SPECVIEWPERF_STAGE_RETRY class=MEDIA")


class Teardown(StageCase):
    """The licence: the copy lives only for the run, and the teardown records its deletion."""

    def test_a_pass_leaves_neither_the_archive_nor_the_overlay(self):
        self.go("lanea")
        self.assertTrue(self.archive_gone())
        self.assertTrue(self.overlay_gone())
        self.assertTrue(self.guest_stopped())

    def test_the_deletions_are_recorded_in_the_log(self):
        self.go("lanea")
        lines = self.p.stdout.splitlines()
        self.assertIn("SPEC_COPY_DELETED removed=archive", lines)
        self.assertIn("SPEC_COPY_DELETED removed=overlay", lines)

    def test_a_failed_run_leaves_neither_the_archive_nor_the_overlay(self):
        self.go("lanea", **{"viewset__creo-04": "fail_subtest"})
        self.assertTrue(self.archive_gone())
        self.assertTrue(self.overlay_gone())

    def test_a_setup_failure_before_the_copy_still_deletes_the_archive(self):
        self.go("lanea", tar_rc="3")
        self.assertTrue(self.archive_gone())
        self.assertTrue(self.overlay_gone())

    def test_the_guest_is_stopped_after_every_outcome(self):
        for scn in ({}, {"fetch": "sha"}, {"verify": "corrupt"}, {"viewset__energy-04": "hang"}):
            with self.subTest(scn=scn):
                self.setUp()
                self.go("lanea", **scn)
                self.assertTrue(self.guest_stopped())
                self.assertIn("win_vm stop", self.sb.calls())

    def test_the_archive_is_deleted_right_after_the_copy_not_only_at_the_end(self):
        self.go("lanea")
        stage = self.stage_log()
        self.assertLess(stage.index("SPEC_COPY_DELETED removed=archive"), stage.index("stage=boot_run"))


class Verdicts(StageCase):
    """THE GATE IS THE GUEST PASSING: every kind of failed viewset is a FAIL, attributed to kayfabe with the host
    reference green; the other viewsets still run."""

    def test_a_failed_subtest_is_a_fail_and_the_others_still_run(self):
        self.go("laneb", **{"viewset__catia-07": "fail_subtest"})
        self.assertEqual(self.p.returncode, 1)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")
        self.assertEqual(self.results()["catia-07"], ("FAIL", "SUBTEST"))
        self.assertEqual([n for n, (r, _) in self.results().items() if r == "PASS"], ["medical-04", "maya-07", "solidworks-08", "unreal_engine-01"])
        self.assertIn("catia_07 --- FAILED", self.p.stdout)
        self.assertEqual(self.rows["catia-07"]["subtests"], "2/3")
        self.assertEqual(self.rows["catia-07"]["failed"], "02_B")

    def test_a_guest_reset_under_a_completed_viewset_is_a_fail(self):
        self.go("lanea", **{"viewset__creo-04": "reset"})
        self.assertEqual(self.results()["creo-04"], ("FAIL", "RESET"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")

    def test_a_viewset_refused_by_spec_validation_is_a_fail_never_excluded(self):
        self.go("lanea", **{"viewset__enscape-01": "refused"})
        self.assertEqual(self.results()["enscape-01"], ("FAIL", "REFUSED"))
        self.assertEqual(self.p.returncode, 1)
        self.assertIn("enscape_01 --- FAILED", self.p.stdout)

    def test_a_viewset_that_hangs_past_its_budget_is_a_fail_with_the_cli_killed(self):
        self.go("laneb", **{"viewset__medical-04": "hang"})
        self.assertEqual(self.results()["medical-04"], ("FAIL", "TIMEOUT"))
        self.assertIn("kill medical-04", self.sb.calls())
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")
        self.assertEqual(self.results()["maya-07"][0], "PASS")

    def test_a_viewset_budget_is_its_own_not_the_time_left(self):
        self.go("laneb", **{"viewset__medical-04": "hang"})
        # 1200 s budget plus the 300 s settle: the clock must have moved at least the budget, and not much more than it
        self.assertGreaterEqual(self.sb.clock(), 1200 + 300)

    def test_a_guest_that_dies_during_a_viewset_is_a_fail_and_the_next_viewset_gets_a_fresh_boot(self):
        self.go("lanea", **{"viewset__creo-04": "dead"})
        self.assertEqual(self.results()["creo-04"], ("FAIL", "GUEST"))
        self.assertEqual(self.results()["blender-01"][0], "PASS")
        self.assertEqual(self.results()["enscape-01"][0], "PASS")
        kf3_boots = [r for r in self.sb.runs() if "nokf3=0" in r]
        self.assertEqual(len(kf3_boots), 2)
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")

    def test_a_fresh_boot_waits_for_the_settle_time_again(self):
        self.go("lanea", **{"viewset__energy-04": "dead"})
        settles = [l for l in self.stage_log().splitlines() if "settle_wait_s=" in l]
        self.assertEqual(len(settles), 2)
        self.assertTrue(all(int(l.split("=")[1]) >= 290 for l in settles), settles)

    def test_the_boots_of_a_dying_guest_are_limited(self):
        self.go("laneb", **{"viewset__medical-04": "dead", "viewset__maya-07": "dead", "viewset__catia-07": "dead"})
        kf3_boots = [r for r in self.sb.runs() if "nokf3=0" in r]
        self.assertEqual(len(kf3_boots), 3)
        self.assertEqual(self.results()["solidworks-08"][0], "NOT_RUN")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")

    def test_a_guest_that_dies_every_time_is_boot_limited_and_every_row_is_a_fail_or_not_run(self):
        self.go("laneb", **{"viewset__%s" % n: "dead" for n in LANE_B})
        self.assertEqual(len([r for r in self.sb.runs() if "nokf3=0" in r]), 3)
        self.assertEqual([self.results()[n][0] for n in LANE_B], ["FAIL", "FAIL", "FAIL", "NOT_RUN", "NOT_RUN"])
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")

    def test_a_viewset_that_reset_the_guest_is_followed_by_a_fresh_boot(self):
        self.go("lanea", **{"viewset__energy-04": "reset"})
        self.assertEqual(self.results()["energy-04"], ("FAIL", "RESET"))
        self.assertEqual(len([r for r in self.sb.runs() if "nokf3=0" in r]), 2)
        self.assertEqual(self.results()["creo-04"][0], "PASS")

    def test_a_viewset_killed_for_its_budget_is_followed_by_a_fresh_boot(self):
        self.go("laneb", **{"viewset__medical-04": "hang"})
        self.assertEqual(len([r for r in self.sb.runs() if "nokf3=0" in r]), 2)
        settles = [l for l in self.stage_log().splitlines() if "settle_wait_s=" in l]
        self.assertEqual(len(settles), 2)

    def test_a_clean_run_never_reboots_the_guest_between_viewsets(self):
        self.go("laneb")
        self.assertEqual(len([r for r in self.sb.runs() if "nokf3=0" in r]), 1)

    def test_a_launch_the_guest_script_refuses_is_a_retry_operator_never_a_guest_failure(self):
        self.go("lanea", **{"viewset__energy-04": "launch_fail"})
        self.assertEqual(self.results()["energy-04"], ("NOT_RUN", "LAUNCH"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=OPERATOR")

    def test_a_launch_that_gets_no_answer_is_the_guests_failure(self):
        self.go("lanea", **{"viewset__energy-04": "launch_silent"})
        self.assertEqual(self.results()["energy-04"], ("FAIL", "GUEST"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")

    def test_a_guest_that_stops_answering_is_a_fail(self):
        self.go("laneb", **{"viewset__maya-07": "silent"})
        self.assertEqual(self.results()["maya-07"], ("FAIL", "GUEST"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")

    def test_a_kf3_boot_that_fails_after_the_gate_booted_the_same_image_is_a_guest_failure(self):
        self.go("lanea", **{"boot_fail__run2": "refused"})
        self.assertEqual(self.results()["energy-04"], ("FAIL", "GUEST"))
        self.assertEqual([r for n, (r, _) in self.results().items() if n != "energy-04"], ["NOT_RUN"] * 3)
        self.assertEqual((self.p.returncode, self.verdict), (1, "SPECVIEWPERF_STAGE_FAIL"))

    def test_the_vast_bar1_refusal_is_the_documented_exception_not_a_fail(self):
        self.go("lanea", **{"boot_fail__run2": "bar1"})
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=BAR1_EXCEPTION")
        self.assertEqual(self.p.returncode, 3)

    def test_a_red_host_reference_after_a_failure_makes_it_a_retry(self):
        self.go("lanea", **{"viewset__creo-04": "fail_subtest", "cup8_fail_after": "1"})
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=HOSTREF")
        self.assertEqual(self.results()["creo-04"][0], "FAIL")

    def test_a_red_host_reference_after_a_pass_is_not_a_pass(self):
        self.go("lanea", cup8_fail_after="1")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=HOSTREF")
        self.assertNotIn("SPECVIEWPERF_STAGE_PASS", self.p.stdout)

    def test_a_new_host_xid_over_a_run_that_passed_is_a_fail(self):
        self.go("lanea", xid_new="1")
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")
        self.assertEqual({r["result"] for r in self.rows.values()}, {"PASS"})

    def test_missing_reset_counters_are_a_retry_not_a_pass(self):
        self.go("lanea", **{"viewset__energy-04": "no_tdr"})
        self.assertEqual(self.results()["energy-04"], ("NOT_RUN", "NO_TDR"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TRANSPORT")

    def test_reset_counters_missing_after_an_end_record_are_a_retry_too(self):
        self.go("lanea", **{"viewset__energy-04": "no_tdr_only"})
        self.assertEqual(self.results()["energy-04"], ("NOT_RUN", "NO_TDR"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TRANSPORT")

    def test_a_refused_launch_in_lane_b_is_a_retry_with_exit_3(self):
        self.go("laneb", **{"viewset__medical-04": "launch_fail"})
        self.assertEqual(self.results()["medical-04"], ("NOT_RUN", "LAUNCH"))
        self.assertEqual((self.p.returncode, self.verdict), (3, "SPECVIEWPERF_STAGE_RETRY class=OPERATOR"))

    def test_the_exit_code_of_a_fail_is_1_and_of_a_retry_is_3(self):
        self.go("lanea", **{"viewset__creo-04": "fail_subtest"})
        self.assertEqual(self.p.returncode, 1)
        self.go("lanea", fetch="sha")
        self.assertEqual(self.p.returncode, 3)


class Gather(StageCase):
    """The harness trap of the first run after a boot: wait, then once more; a second miss is a RETRY."""

    def test_one_gather_failure_is_retried_after_a_wait_and_passes(self):
        self.go("lanea", **{"viewset__energy-04": "gather_once"})
        self.assertEqual(self.results()["energy-04"], ("PASS", "-"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_PASS")
        self.assertIn("120", self.sb.read("sleeps.log").split())
        self.assertEqual(self.sb.read("launches.energy-04").strip(), "2")

    def test_a_second_gather_failure_is_not_run_and_a_retry_never_a_pass(self):
        self.go("lanea", **{"viewset__energy-04": "gather"})
        self.assertEqual(self.results()["energy-04"], ("NOT_RUN", "GATHER"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=OPERATOR")
        self.assertEqual(self.sb.read("launches.energy-04").strip(), "2")

    def test_the_retry_is_once_not_forever(self):
        self.go("lanea", **{"viewset__energy-04": "gather"})
        self.assertEqual(self.sb.read("launches.energy-04").strip(), "2")

    def test_only_the_gather_failure_is_retried_a_real_failure_is_not(self):
        self.go("lanea", **{"viewset__energy-04": "fail_subtest"})
        self.assertEqual(self.sb.read("launches.energy-04").strip(), "1")
        self.assertEqual(self.results()["energy-04"][0], "FAIL")


class Settle(StageCase):
    def test_no_viewset_starts_sooner_than_five_minutes_after_the_boot(self):
        self.go("lanea")
        self.assertGreaterEqual(self.sb.clock(), 300 + 4 * 60)
        self.assertIn("300", self.sb.read("sleeps.log").split())

    def test_the_wait_is_the_lane_table_value(self):
        import json

        self.assertGreaterEqual(json.loads((fk.BENCH / "specviewperf/lanes.json").read_text())["settle_s"], 300)

    def test_the_settle_wait_counts_from_the_boot_not_from_the_stage_start(self):
        self.go("lanea")
        waits = [int(l.split("=")[1]) for l in self.stage_log().splitlines() if "settle_wait_s=" in l]
        self.assertEqual(len(waits), 1)
        self.assertGreaterEqual(waits[0], 295)


class TimeGuard(StageCase):
    """No viewset starts without the time for it; a viewset cut short by the clock is a RETRY, never a verdict."""

    def test_a_viewset_without_time_is_not_started_and_the_lane_is_a_retry_time(self):
        # energy-04 needs 1.25 x 323 s + the 600 s tail; creo-04 needs 1554 s + 600 s. Give the first and not the second.
        self.go("lanea", budget=300 + 1100 + 700 + 200, **{"wall__energy-04": 100})
        self.assertEqual(self.results()["energy-04"][0], "PASS")
        for n in ("creo-04", "blender-01", "enscape-01"):
            self.assertEqual(self.results()[n], ("NOT_RUN", "TIME"), n)
        self.assertEqual((self.p.returncode, self.verdict), (3, "SPECVIEWPERF_STAGE_RETRY class=TIME"))
        self.assertNotIn("launch creo-04", self.sb.calls())

    def test_a_skipped_viewset_never_launches_in_the_guest(self):
        self.go("lanea", budget=300 + 1100 + 300)
        self.assertNotIn("launch creo-04", self.sb.calls())
        self.assertNotIn("launch blender-01", self.sb.calls())

    def test_a_viewset_cut_by_the_clock_that_does_not_end_is_a_retry_time_not_a_fail(self):
        # 1700 s left at its start: usable = 1100 s, above need (404 s) and below its budget (1200 s): mode cut
        self.go("lanea", budget=300 + 1700, **{"viewset__energy-04": "hang"})
        self.assertEqual(self.results()["energy-04"], ("NOT_RUN", "TIME"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_RETRY class=TIME")
        self.assertIn("kill energy-04", self.sb.calls())

    def test_a_cut_viewset_ends_the_stage_in_a_retry_with_exit_3_and_no_fail_marker(self):
        self.go("lanea", budget=300 + 1700, **{"viewset__energy-04": "hang"})
        self.assertEqual(self.p.returncode, 3)
        self.assertNotIn("--- FAILED", self.p.stdout)
        self.assertNotIn("SPECVIEWPERF_STAGE_FAIL", self.p.stdout)

    def test_a_viewset_with_its_full_budget_that_hangs_is_a_fail(self):
        self.go("lanea", budget=40000, **{"viewset__energy-04": "hang"})
        self.assertEqual(self.results()["energy-04"], ("FAIL", "TIMEOUT"))

    def test_the_budget_of_a_viewset_after_a_fresh_boot_comes_from_the_time_left_after_the_boot(self):
        # energy-04 kills the guest; creo-04 needs a boot and the 300 s settle first, so its budget is what is left THEN
        self.go("lanea", budget=300 + 2600, **{"viewset__energy-04": "dead"})
        run = [l for l in self.stage_log().splitlines() if "run creo-04 (budget" in l]
        self.assertEqual(len(run), 1, self.stage_log())
        budget = int(run[0].split("budget ")[1].split("s")[0])
        self.assertLess(budget, 2000 - 100)
        self.assertIn("mode cut", run[0])

    def test_a_viewset_that_no_longer_fits_after_the_boot_is_not_started(self):
        self.go("lanea", budget=300 + 2300, **{"viewset__energy-04": "dead"})
        self.assertEqual(self.results()["creo-04"], ("NOT_RUN", "TIME"))
        self.assertNotIn("launch creo-04", self.sb.calls())

    def test_enscape_that_no_longer_fits_after_the_fresh_boot_of_a_dead_blender_is_not_started(self):
        # enscape-01 needs 3,152 s: it fits before the boot (3,380 left) and not after it and the 300 s settle
        self.go("lanea", budget=3800, **{"viewset__blender-01": "dead"})
        self.assertEqual(self.results()["enscape-01"], ("NOT_RUN", "TIME"))
        self.assertNotIn("launch enscape-01", self.sb.calls())
        self.assertEqual(self.results()["creo-04"][0], "PASS")

    def test_a_viewset_that_cannot_fit_is_not_worth_a_boot_after_a_dead_guest(self):
        # energy-04 takes the guest down; creo-04 needs 2,153 s and 2,000 are left: no second kf3 boot for nothing
        self.go("lanea", budget=300 + 2000, **{"viewset__energy-04": "dead"})
        self.assertEqual(len([r for r in self.sb.runs() if "nokf3=0" in r]), 1)
        self.assertEqual(self.results()["creo-04"], ("NOT_RUN", "TIME"))

    def test_a_viewset_that_cannot_fit_is_not_worth_a_fresh_boot_after_a_reset(self):
        self.go("lanea", budget=300 + 2000, **{"viewset__energy-04": "reset"})
        self.assertEqual(len([r for r in self.sb.runs() if "nokf3=0" in r]), 1)
        self.assertEqual(self.results()["creo-04"], ("NOT_RUN", "TIME"))
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")

    def test_the_stage_reserves_the_tail_for_the_graceful_stop(self):
        # exactly the need of energy-04 and no tail: not started
        self.go("lanea", budget=300 + 404 + 100)
        self.assertEqual(self.results()["energy-04"], ("NOT_RUN", "TIME"))

    def test_a_fail_beats_a_time_shortfall(self):
        self.go("lanea", budget=300 + 1100 + 700 + 200, **{"viewset__energy-04": "fail_subtest", "wall__energy-04": 100})
        self.assertEqual(self.verdict, "SPECVIEWPERF_STAGE_FAIL")


class Scores(StageCase):
    """Scores never leave: the vendor result directory is never opened, and only status lines are kept."""

    def test_a_cli_log_with_a_score_line_leaves_no_score_in_the_stream_or_the_evidence(self):
        self.go("lanea", **{"viewset__energy-04": "scoreleak"})
        evidence = "".join(p.read_text() for p in (self.sb.prov / (self.sb.tag + "_spec")).glob("*") if p.is_file())
        for text in (self.p.stdout, evidence):
            self.assertNotIn("123.45", text)
            self.assertNotIn("Composite", text)
            self.assertNotIn("results", text.replace("results and logs", "").replace("SPEC_NOTE", "") if text is evidence else text)

    def test_the_status_extract_of_the_cli_log_holds_only_status_lines(self):
        self.go("lanea", **{"viewset__energy-04": "scoreleak"})
        extract = self.sb.read("prov/%s_spec/energy-04.cli.txt" % self.sb.tag)
        self.assertIn("[workload: energy-04 / subtest: 01_A] Completed", extract)
        self.assertNotIn("Composite", extract)
        self.assertNotIn("C:\\spec", extract)
        self.assertNotIn("Run complete", extract)

    def test_the_raw_guest_output_stays_out_of_the_evidence_directory_when_a_private_dir_is_given(self):
        priv = self.sb.root / "priv"
        p = subprocess.run(self.sb.command(), env=self.sb.env(KF_SPEC_PRIVATE=str(priv)), capture_output=True, text=True,
                           timeout=240, stdin=subprocess.DEVNULL)
        self.sb.cleanup()
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertTrue((priv / "energy-04/collect.txt").exists())
        self.assertFalse((self.sb.prov / (self.sb.tag + "_spec/work")).exists())

    def test_the_guest_script_never_opens_the_vendor_result_directory(self):
        text = (fk.BENCH / "windows/spec/spec_guest.ps1").read_text() + (fk.BENCH / "windows/spec/spec_run.ps1").read_text()
        code = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))
        for forbidden in ("Get-Content $res", "Get-ChildItem $res", "result_", "results.json", "Copy-Item"):
            self.assertNotIn(forbidden, code)


class Wrapper(StageCase):
    """The wrapper keeps the whole output private and lets only the strict status lines through, as they arrive."""

    def wrap(self, lane="lanea", **scn):
        for k, v in scn.items():
            self.sb.set(k.replace("__", "."), v)
        self.p = self.sb.run(lane, wrapper=True)
        return self.p

    def test_the_streamed_log_holds_only_status_lines_and_the_verdict(self):
        p = self.wrap("lanea")
        self.assertEqual(p.returncode, 0, p.stdout)
        lines = p.stdout.splitlines()
        self.assertEqual(lines[-2:], ["SPECVIEWPERF_STAGE_PASS", "SPECVIEWPERF_STAGE_EXIT rc=0"])
        for line in lines:
            self.assertNotRegex(line, r"^SPEC_NOTE")
            self.assertRegex(line, PUBLIC)
        self.assertEqual(len([l for l in lines if l.startswith("SPEC_ROW ")]), 4)

    def test_the_first_line_is_the_start_marker_with_the_lane_and_the_last_is_the_exit_line(self):
        p = self.wrap("laneb")
        lines = p.stdout.splitlines()
        self.assertEqual(lines[0], "SPEC_STAGE_START lane=laneb")
        self.assertRegex(lines[-1], r"^SPECVIEWPERF_STAGE_EXIT rc=[0-9]+$")

    def test_a_lane_that_is_not_one_is_never_echoed_in_the_start_marker(self):
        lane = self.sb.root / "lane6.sh"
        lane.write_text("echo SPECVIEWPERF_STAGE_PASS\n")
        env = self.sb.env(lane="lanea; echo score=1")
        env["KF_SPECVIEWPERF_LANE"] = str(lane)
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(p.stdout.splitlines()[0], "SPEC_STAGE_START lane=none")
        self.assertNotIn("score", p.stdout)

    def test_a_killed_run_still_has_its_start_marker(self):
        self.sb.set("block.creo-04", "1")
        env = self.sb.env()
        env["KF_SPECVIEWPERF_LANE"] = str(self.sb.src / "scripts/bench/box/specviewperf_stage.sh")
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        child = subprocess.Popen(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                                 stdout=subprocess.PIPE, text=True, start_new_session=True, stdin=subprocess.DEVNULL)
        deadline = time.time() + 60
        while time.time() < deadline and not (self.sb.root / "blocked").exists():
            time.sleep(0.05)
        os.killpg(child.pid, signal.SIGKILL)
        out, _ = child.communicate(timeout=60)
        self.sb.cleanup()
        self.assertEqual(out.splitlines()[0], "SPEC_STAGE_START lane=lanea")

    def test_the_private_log_holds_the_whole_output_with_its_notes(self):
        self.wrap("lanea")
        log = (self.sb.root / ("private/specviewperf_%s.log" % self.sb.tag)).read_text()
        self.assertIn("SPEC_NOTE", log)
        self.assertIn("SPEC_ROW name=energy-04", log)
        self.assertEqual((self.sb.root / ("private/specviewperf_%s.log" % self.sb.tag)).stat().st_mode & 0o777, 0o600)

    def test_the_private_directory_holds_the_raw_guest_output_and_the_evidence_directory_does_not(self):
        self.wrap("lanea")
        self.assertTrue((self.sb.root / ("private/spec_%s/energy-04/collect.txt" % self.sb.tag)).exists())
        self.assertFalse((self.sb.prov / (self.sb.tag + "_spec/work")).exists())

    def test_the_private_log_and_its_directory_are_not_readable_by_others(self):
        self.wrap("lanea")
        self.assertEqual((self.sb.root / "private").stat().st_mode & 0o777, 0o700)

    def test_the_private_log_file_is_mode_0600(self):
        self.wrap("lanea")
        self.assertEqual((self.sb.root / ("private/specviewperf_%s.log" % self.sb.tag)).stat().st_mode & 0o777, 0o600)

    def test_the_wrapper_exit_code_is_the_stages(self):
        self.assertEqual(self.wrap("lanea", **{"viewset__creo-04": "fail_subtest"}).returncode, 1)
        self.assertEqual(self.wrap("lanea", fetch="sha").returncode, 3)

    def test_a_row_line_with_trailing_text_is_not_a_status_line(self):
        lane = self.sb.root / "lane4.sh"
        lane.write_text("echo 'SPEC_ROW name=energy-04 result=PASS subtests=3/3 reason=- failed=- note=secret'\necho SPECVIEWPERF_STAGE_PASS\n")
        env = self.sb.env()
        env["KF_SPECVIEWPERF_LANE"] = str(lane)
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                           capture_output=True, text=True, timeout=60)
        self.assertNotIn("secret", p.stdout)
        self.assertNotIn("SPEC_ROW", p.stdout)

    def test_a_class_that_only_starts_like_a_listed_one_is_no_verdict(self):
        for cls in ("OPERATORS", "MEDIA_X", "TIMEOUT"):
            with self.subTest(cls):
                lane = self.sb.root / ("lane7_%s.sh" % cls)
                lane.write_text("echo 'SPECVIEWPERF_STAGE_RETRY class=%s'\nexit 3\n" % cls)
                env = self.sb.env()
                env["KF_SPECVIEWPERF_LANE"] = str(lane)
                env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
                p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                                   capture_output=True, text=True, timeout=60)
                self.assertEqual(p.stdout.splitlines()[1], "SPECVIEWPERF_STAGE_RETRY class=NO_VERDICT")

    def test_a_lane_value_with_a_newline_is_not_echoed_in_the_start_marker(self):
        lane = self.sb.root / "lane8.sh"
        lane.write_text("echo SPECVIEWPERF_STAGE_PASS\n")
        env = self.sb.env()
        env["KF_SPEC_LANE"] = "lanea\nSPEC_ROW name=energy-04 result=PASS subtests=1/1 reason=- failed=-"
        env["KF_SPECVIEWPERF_LANE"] = str(lane)
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(p.stdout.splitlines()[0], "SPEC_STAGE_START lane=none")
        self.assertNotIn("SPEC_ROW", p.stdout)

    def test_no_raw_guest_file_is_anywhere_in_the_evidence_directory_of_a_wrapped_run(self):
        self.wrap("lanea")
        names = {p.name for p in (self.sb.prov / (self.sb.tag + "_spec")).rglob("*") if p.is_file()}
        self.assertNotIn("collect.txt", names)
        self.assertIn("energy-04.cli.txt", names)

    def test_a_verdict_class_with_digits_that_is_not_in_the_list_is_no_verdict(self):
        lane = self.sb.root / "lane5.sh"
        lane.write_text("echo 'SPECVIEWPERF_STAGE_RETRY class=SCORE_123'\nexit 3\n")
        env = self.sb.env()
        env["KF_SPECVIEWPERF_LANE"] = str(lane)
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(p.stdout.splitlines()[1], "SPECVIEWPERF_STAGE_RETRY class=NO_VERDICT")

    def test_a_count_line_with_extra_text_is_not_a_status_line(self):
        lane = self.sb.root / "lane2.sh"
        lane.write_text("echo 'SPEC_STAGE_COUNTS pass=1 fail=0 not_run=0 of=1 total_fps=60'\necho 'SPEC_STAGE_LIST lane=lanea n=4 x'\necho SPECVIEWPERF_STAGE_PASS\n")
        env = self.sb.env()
        env["KF_SPECVIEWPERF_LANE"] = str(lane)
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                           capture_output=True, text=True, timeout=60)
        self.assertNotIn("fps", p.stdout)
        self.assertNotIn("SPEC_STAGE_LIST", p.stdout)

    def test_a_verdict_class_outside_the_fixed_list_is_no_verdict(self):
        lane = self.sb.root / "lane3.sh"
        lane.write_text("echo 'SPECVIEWPERF_STAGE_RETRY class=SCORE'\nexit 3\n")
        env = self.sb.env()
        env["KF_SPECVIEWPERF_LANE"] = str(lane)
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(p.stdout.splitlines()[1], "SPECVIEWPERF_STAGE_RETRY class=NO_VERDICT")

    def test_a_scoreleak_in_the_guest_output_never_reaches_the_stream(self):
        p = self.wrap("lanea", **{"viewset__energy-04": "scoreleak"})
        self.assertNotIn("123.45", p.stdout)
        self.assertNotIn("Composite", p.stdout)

    def test_a_fail_prints_its_row_the_failed_marker_and_the_fail_verdict(self):
        p = self.wrap("lanea", **{"viewset__creo-04": "fail_subtest"})
        self.assertEqual(p.returncode, 1)
        self.assertIn("SPEC_ROW name=creo-04 result=FAIL subtests=2/3 reason=SUBTEST failed=02_B", p.stdout)
        self.assertIn("creo_04 --- FAILED", p.stdout)
        self.assertIn("SPECVIEWPERF_STAGE_FAIL\n", p.stdout)

    def test_a_line_with_extra_fields_is_not_a_status_line(self):
        lane = self.sb.root / "lane.sh"
        lane.write_text(
            "echo 'SPEC_ROW name=energy-04 result=PASS subtests=3/3 reason=- failed=- score=99.5'\n"
            "echo 'SPEC_ROW name=energy-04 result=PASS subtests=3/3 reason=- failed=-'\n"
            "echo 'SPEC_STAGE_COUNTS pass=1 fail=0 not_run=0 of=1 fps=60'\n"
            "echo 'composite 12.5'\n"
            "echo SPECVIEWPERF_STAGE_PASS\nexit 0\n"
        )
        env = self.sb.env()
        env["KF_SPECVIEWPERF_LANE"] = str(lane)
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        p = subprocess.run(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(p.stdout.splitlines()[1], "SPEC_ROW name=energy-04 result=PASS subtests=3/3 reason=- failed=-")
        self.assertNotIn("score", p.stdout)
        self.assertNotIn("fps", p.stdout)
        self.assertNotIn("composite", p.stdout)

    def test_a_killed_run_has_already_streamed_the_rows_that_finished(self):
        self.sb.set("viewset.creo-04", "pass")
        self.sb.set("block.creo-04", "1")
        env = self.sb.env()
        env["KF_SPECVIEWPERF_LANE"] = str(self.sb.src / "scripts/bench/box/specviewperf_stage.sh")
        env["KF_CI_PRIVATE_DIR"] = str(self.sb.root / "private")
        child = subprocess.Popen(["bash", str(fk.HERE / "vast_stage_specviewperf.sh"), self.sb.tag, str(self.sb.prov)], env=env,
                                 stdout=subprocess.PIPE, text=True, start_new_session=True, stdin=subprocess.DEVNULL)
        seen = []
        deadline = time.time() + 60
        while time.time() < deadline and not (self.sb.root / "blocked").exists():
            time.sleep(0.05)
        self.assertTrue((self.sb.root / "blocked").exists(), "the fake guest never blocked")
        os.killpg(child.pid, signal.SIGTERM)
        out, _ = child.communicate(timeout=60)
        self.sb.cleanup()
        seen = out.splitlines()
        self.assertIn("SPEC_ROW name=energy-04 result=PASS subtests=3/3 reason=- failed=-", seen)
        self.assertNotIn("SPECVIEWPERF_STAGE_PASS", seen)


class Interrupted(StageCase):
    def test_a_term_signal_ends_in_retry_operator_stops_the_guest_and_deletes_the_copy(self):
        self.sb.set("block.creo-04", "1")
        env = self.sb.env()
        child = subprocess.Popen(self.sb.command(), env=env, stdout=subprocess.PIPE, text=True, start_new_session=True,
                                 stdin=subprocess.DEVNULL)
        deadline = time.time() + 60
        while time.time() < deadline and not (self.sb.root / "blocked").exists():
            time.sleep(0.05)
        self.assertTrue((self.sb.root / "blocked").exists())
        os.killpg(child.pid, signal.SIGTERM)
        out, _ = child.communicate(timeout=60)
        self.sb.cleanup()
        self.assertEqual(child.returncode, 3)
        self.assertEqual(out.splitlines()[-1], "SPECVIEWPERF_STAGE_RETRY class=OPERATOR")
        self.assertTrue(self.archive_gone())
        self.assertTrue(self.overlay_gone())
        self.assertIn("win_vm stop", self.sb.calls())
        self.assertNotIn("SPECVIEWPERF_STAGE_PASS", out)

    def test_the_rows_of_the_viewsets_that_finished_were_printed_before_the_signal(self):
        self.sb.set("block.blender-01", "1")
        child = subprocess.Popen(self.sb.command(), env=self.sb.env(), stdout=subprocess.PIPE, text=True,
                                 start_new_session=True, stdin=subprocess.DEVNULL)
        deadline = time.time() + 60
        while time.time() < deadline and not (self.sb.root / "blocked").exists():
            time.sleep(0.05)
        os.killpg(child.pid, signal.SIGTERM)
        out, _ = child.communicate(timeout=60)
        self.sb.cleanup()
        rows, order = fk.rows_of(out)
        self.assertEqual(order, ["energy-04", "creo-04"])
        self.assertEqual(out.splitlines()[-1], "SPECVIEWPERF_STAGE_RETRY class=OPERATOR")
        self.assertEqual(child.returncode, 3)


if __name__ == "__main__":
    unittest.main()
