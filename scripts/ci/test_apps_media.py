#!/usr/bin/env python3
"""Linux apps media on a fresh Worker box (2026-11-10), GPU-free.

A fresh box has no bundle, no guest image and no PROVISIONED marker. The payload must start the stage with
KF_APPS_PROVISION=1 and reserve the time that provisioning and the matrix need; a failed provisioning step must be a
MEDIA retry (rerun on a fresh box), never an app FAIL and never a graded matrix.

FALSIFIER (written before the code): the payload plans the apps stage without KF_APPS_PROVISION=1, or lets the
environment weaken it; the stage reserves no more than the old 60 minutes; a BUILD_*=FAIL or SETUP_*=FAIL line, a
non-zero build exit or a failed guest setup ends in anything but `APPS_STAGE_RETRY class=MEDIA`; the matrix runs on
media that did not build; or a successful provisioning does not reach the matrix.
"""

import contextlib
import os
import re
import subprocess
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

import test_vast_payload_apps as apps
import test_vast_payload_v7 as v7

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
EXPECTED = [
    line.strip()
    for line in (ROOT / "scripts/apps/expected_rows.txt").read_text().splitlines()
    if line.strip() and not line.startswith("#")
]
EXPECTED_BUILD = [r for r in EXPECTED if r.startswith("BUILD_")]
EXPECTED_SETUP = [r for r in EXPECTED if r.startswith("SETUP_")]
PROVISION_MINUTES = 29  # measured, see the comment above APPS_MIN_REMAINING_MS
MATRIX_CAP_S = 3000  # KF_APPS_BUDGET_S default of the stage


class PayloadPlansProvisioning(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def go(self, env=None, soft=300, windows="0", probe=None, names=None, text=None):
        names = apps.NAMES if names is None else names
        extra = {"KF_TIER": "middle", "KF_LINUX_APPS": "1"}
        extra.update(env or {})
        self.commands = []
        original = self.m.watched

        def spy(command, root, name):
            if any(str(part).endswith("apps_gate.sh") for part in command):
                self.commands.append(list(command))
            return original(command, root, name)

        with (
            patch.object(self.m, "watched", side_effect=spy),
            patch.object(self.m, "apps_gate_list", return_value=names),
        ):
            return self._main_fixture(
                windows=windows,
                soft_ms=soft * 60000,
                extra_env=extra,
                stage_logs={"apps": (text or apps.log(names=names), 0)},
                probe=probe,
            )

    def test_the_apps_stage_is_started_with_provisioning_on(self):
        rc, result, calls = self.go()
        self.assertEqual(rc, 0, result)
        self.assertIn("apps", calls)
        (command,) = self.commands
        index = command.index("KF_APPS_PROVISION=1")
        self.assertEqual(command[index - 1], "env")
        self.assertTrue(command[index + 1] == "bash" and command[index + 2].endswith("apps_gate.sh"))

    def test_the_provisioning_flag_is_exactly_one_and_not_repeated(self):
        self.go()
        (command,) = self.commands
        self.assertEqual([p for p in command if p.startswith("KF_APPS_PROVISION")], ["KF_APPS_PROVISION=1"])

    def test_the_environment_cannot_weaken_provisioning(self):
        for value in ("0", "", "no"):
            with self.subTest(value):
                rc, result, _ = self.go(env={"KF_APPS_PROVISION": value})
                self.assertEqual(rc, 0, result)
                (command,) = self.commands
                self.assertIn("KF_APPS_PROVISION=1", command)

    def test_the_ambient_flag_is_consumed_before_any_child_runs(self):
        snapshots = []
        rc, _, _ = self.go(env={"KF_APPS_PROVISION": "0"}, probe=snapshots.append)
        self.assertEqual(rc, 0)
        self.assertTrue(snapshots)
        self.assertTrue(all("KF_APPS_PROVISION" not in env for env in snapshots))

    def test_the_direct_call_consumes_the_flag_too(self):
        env = {"KF_LINUX_APPS": "1", "KF_APPS_PROVISION": "1"}
        self.assertIs(self.m.linuxapps_requested(env, "middle"), True)
        self.assertNotIn("KF_APPS_PROVISION", env)

    def test_provisioning_is_not_a_worker_input(self):
        # no SPEC row: the box's environment contract drops it, and the Worker has no field for it
        self.assertNotIn("KF_APPS_PROVISION", (HERE / "vast_report.py").read_text())
        for path in sorted((ROOT / "ci/vast-broker/src").glob("*.js")):
            self.assertNotIn("KF_APPS_PROVISION", path.read_text(), path.name)

    def test_no_stage_means_no_provisioning_command(self):
        with patch.object(self.m, "apps_gate_list", return_value=apps.NAMES):
            for tier in ("small", "adhoc"):
                self.commands = []
                original = self.m.watched

                def spy(command, root, name):
                    if any(str(part).endswith("apps_gate.sh") for part in command):
                        self.commands.append(list(command))
                    return original(command, root, name)

                with patch.object(self.m, "watched", side_effect=spy):
                    rc, result, calls = self._main_fixture(
                        soft_ms=300 * 60000, extra_env={"KF_TIER": tier}
                    )
                self.assertEqual(rc, 0, result)
                self.assertEqual(self.commands, [])
                self.assertNotIn("apps", calls)


class PayloadReservesTheMediaWall(unittest.TestCase):
    setUp = PayloadPlansProvisioning.setUp
    _main_fixture = v7.V7Tests._main_fixture
    go = PayloadPlansProvisioning.go

    def test_the_reserve_covers_provisioning_plus_the_matrix_cap(self):
        need = (PROVISION_MINUTES * 60 + MATRIX_CAP_S) * 1000
        self.assertGreaterEqual(self.m.APPS_MIN_REMAINING_MS, need)

    def test_the_reserve_is_not_the_old_sixty_minutes(self):
        self.assertGreater(self.m.APPS_MIN_REMAINING_MS, 60 * 60000)

    def test_apps_alone_do_not_start_just_under_the_reserve(self):
        minutes = self.m.APPS_MIN_REMAINING_MS // 60000
        rc, result, calls = self.go(soft=minutes - 1)
        self.assertEqual(rc, 1)
        self.assertNotIn("apps", calls)
        self.assertIn("apps_retry_time", [row["name"] for row in result["suites"]])

    def test_apps_alone_start_with_the_reserve_left(self):
        minutes = self.m.APPS_MIN_REMAINING_MS // 60000
        rc, result, calls = self.go(soft=minutes + 1)
        self.assertEqual(rc, 0, result)
        self.assertIn("apps", calls)

    def test_the_windows_suffix_is_added_on_top(self):
        suffix = (self.m.WINDOWS_MIN_REMAINING_MS + self.m.WINAPPS_MIN_REMAINING_MS["mid"]) // 60000
        minutes = self.m.APPS_MIN_REMAINING_MS // 60000 + suffix
        rc, result, calls = self.go(
            soft=minutes - 1, windows="1", env={"KF_WINAPPS": "mid"}
        )
        self.assertEqual(rc, 1)
        self.assertNotIn("apps", calls)


def build_tree(root):
    """A copy of the pieces apps_gate.sh needs, so the stage can be run with fake build scripts."""
    scripts = root / "scripts"
    shutil.copytree(ROOT / "scripts/kfci", scripts / "kfci", ignore=shutil.ignore_patterns("__pycache__"))
    (scripts / "ci").mkdir()
    shutil.copyfile(ROOT / "scripts/ci/app_policy.py", scripts / "ci/app_policy.py")
    shutil.copytree(ROOT / "scripts/apps", scripts / "apps", ignore=shutil.ignore_patterns("__pycache__", "image"))
    (scripts / "bench/box").mkdir(parents=True)
    shutil.copyfile(ROOT / "scripts/bench/box/apps_gate.sh", scripts / "bench/box/apps_gate.sh")
    profiles = scripts / "bench/windows/appmatrix"
    profiles.mkdir(parents=True)
    shutil.copyfile(ROOT / "scripts/bench/windows/appmatrix/gate_profiles.json", profiles / "gate_profiles.json")
    return scripts


class StageProvisioning(unittest.TestCase):
    """The stage with KF_APPS_PROVISION=1 and fake build/provision scripts: what a failed step turns into."""

    # every expected row (scripts/apps/expected_rows.txt) printed ok, as the real scripts do
    GOOD_BUILD = (
        "".join("echo %s=ok\n" % row for row in EXPECTED_BUILD)
        + "mkdir -p \"$KF_APPS_ROOT\"; echo bundle > \"$KF_APPS_ROOT/bundle.tgz\"\n"
    )
    GOOD_GUEST = (
        "".join("echo '  guest: %s=ok'\n" % row for row in EXPECTED_SETUP) + "echo '  guest: SETUP_SIDE_DONE'\n"
    )

    def run_stage(self, build=None, guest=None, provision="1", marker_first=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = build_tree(root)
            (scripts / "apps/build_bundle.sh").write_text("#!/bin/bash\n" + (build or self.GOOD_BUILD))
            (scripts / "apps/provision_guest_apps.sh").write_text(
                "#!/bin/bash\ntouch \"$KF_APPS_ROOT/guest.ran\"\n" + (guest or self.GOOD_GUEST)
            )
            bench = root / "bench"
            bench.mkdir()
            (bench / "guest.qcow2").write_text("image")
            (bench / "guest.qcow2.STAGED").write_text("image=/x\n")
            (root / "apps").mkdir()
            if marker_first:
                (root / "apps/PROVISIONED").write_text("x")
            (root / "list.txt").write_text("vectorAdd\ndeviceQuery\n")
            (root / "matrix.sh").write_text(
                "#!/bin/bash\nR=%s/apps/results/$2; mkdir -p $R\n" % root
                + "echo MATRIX_RAN >> %s/matrix.ran\n" % root
                + "".join(
                    "echo 'APPRES side=guest app=%s verdict=PASS rc=0 secs=2 quiet=0 note=- boot=b1 guest_xid=0 kf3_lines=1 kf3_refusals=0' >> $R/guest.res\n"
                    % name
                    for name in ("vectorAdd", "deviceQuery")
                )
            )
            env = {
                "PATH": os.environ["PATH"],
                "KF_APPS_LIST": str(root / "list.txt"),
                "KF_APPS_ROOT": str(root / "apps"),
                "KF_APPS_MATRIX": str(root / "matrix.sh"),
                "KF_APPS_BUDGET_S": "30",
                "KF_APPS_POLL_S": "0.2",
                "BENCH_DIR": str(bench),
                "KF_APPS_TEST_MIN_FREE_GB": "0",  # the test machine may have less than the stage's 60 GB
            }
            if provision is not None:
                env["KF_APPS_PROVISION"] = provision
            ran = subprocess.run(
                ["bash", str(scripts / "bench/box/apps_gate.sh"), "ci0abc", str(root / "prov"), "middle"],
                env=env,
                capture_output=True,
                text=True,
                timeout=120,
            )
            log = root / "prov/ci0abc_apps_provision.log"
            return dict(
                rc=ran.returncode,
                out=ran.stdout,
                matrix_ran=(root / "matrix.ran").exists(),
                guest_ran=(root / "apps/guest.ran").exists(),
                marker=(root / "apps/PROVISIONED").exists(),
                log=log.read_text() if log.exists() else "",
            )

    def assert_media_retry(self, got):
        self.assertEqual(got["rc"], 3, got["out"])
        self.assertIn("APPS_STAGE_RETRY class=MEDIA", got["out"])
        self.assertIn("APPS_PROVISION_START", got["out"])
        self.assertNotIn("APPS_STAGE_FAIL", got["out"])
        self.assertNotIn("APPS_STAGE_PASS", got["out"])
        self.assertNotIn("APP_ROW", got["out"])
        self.assertFalse(got["matrix_ran"])
        self.assertFalse(got["marker"])

    def test_good_media_reaches_the_matrix_and_passes(self):
        got = self.run_stage()
        self.assertEqual(got["rc"], 0, got["out"])
        self.assertIn("APPS_PROVISION_END rc=0", got["out"])
        self.assertTrue(got["matrix_ran"])
        self.assertTrue(got["marker"])
        self.assertIn("APPS_STAGE_PASS", got["out"])

    def test_without_the_flag_missing_media_is_still_a_retry_without_building(self):
        got = self.run_stage(provision=None)
        self.assertEqual(got["rc"], 3, got["out"])
        self.assertIn("APPS_STAGE_RETRY class=MEDIA", got["out"])
        self.assertNotIn("APPS_PROVISION_START", got["out"])
        self.assertFalse(got["matrix_ran"])

    def test_a_build_that_exits_nonzero_is_a_media_retry(self):
        self.assert_media_retry(self.run_stage(build="echo BUILD_llama_cpp=FAIL rc=2\nexit 1\n"))

    def test_a_build_fail_line_with_exit_zero_is_a_media_retry(self):
        # the old build_bundle.sh: FAIL lines, a bundle.tgz anyway, exit 0
        self.assert_media_retry(self.run_stage(build=self.GOOD_BUILD + "echo BUILD_llama_cpp=FAIL rc=2\n"))

    def test_a_build_fail_line_with_exit_zero_does_not_start_the_guest_image(self):
        got = self.run_stage(build=self.GOOD_BUILD + "echo BUILD_llama_cpp=FAIL rc=2\n")
        self.assertFalse(got["guest_ran"])

    def test_another_build_fail_line_with_exit_zero_does_not_start_the_guest_image(self):
        got = self.run_stage(build="echo BUILD_vkpeak=FAIL rc=1\n" + self.GOOD_BUILD)
        self.assertFalse(got["guest_ran"])

    def test_a_clean_build_does_start_the_guest_image(self):
        self.assertTrue(self.run_stage()["guest_ran"])

    def test_another_build_fail_line_is_a_media_retry(self):
        self.assert_media_retry(
            self.run_stage(build="echo BUILD_sample_scan=FAIL rc=1\n" + self.GOOD_BUILD)
        )

    def test_the_failed_step_is_named_in_one_bounded_line(self):
        got = self.run_stage(build=self.GOOD_BUILD + "echo BUILD_vkpeak=FAIL rc=1\n")
        self.assertIn("APPS_PROVISION_FAIL steps=BUILD_vkpeak\n", got["out"])

    def test_a_failed_guest_step_is_named_when_the_guest_script_exits_nonzero(self):
        # the real provision_guest_apps.sh exits 5 on a failed row: the stage's output must still name the step
        got = self.run_stage(
            guest=self.GOOD_GUEST.replace("SETUP_hf_model=ok", "SETUP_hf_model=FAIL rc=1") + "exit 5\n"
        )
        self.assert_media_retry(got)
        self.assertIn("APPS_PROVISION_FAIL steps=SETUP_hf_model,missing_SETUP_hf_model\n", got["out"])

    def test_a_failed_build_step_is_named_when_the_build_exits_nonzero(self):
        got = self.run_stage(build=self.GOOD_BUILD + "echo BUILD_vkpeak=FAIL rc=1\nexit 1\n")
        self.assert_media_retry(got)
        self.assertIn("APPS_PROVISION_FAIL steps=BUILD_vkpeak\n", got["out"])

    def test_a_guest_setup_fail_line_is_a_media_retry(self):
        self.assert_media_retry(
            self.run_stage(guest="echo '  guest: SETUP_gguf=FAIL rc=1'\necho '  guest: SETUP_SIDE_DONE'\n")
        )

    def test_a_guest_setup_script_that_exits_nonzero_is_a_media_retry(self):
        self.assert_media_retry(self.run_stage(guest="exit 5\n"))

    def test_ok_lines_and_other_text_are_not_failures(self):
        import sys

        sys.path.insert(0, str(ROOT / "scripts"))
        try:
            from kfci.apps_gate import provision_failures
        finally:
            sys.path.pop(0)
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "p.log"
            log.write_text(
                "BUILD_a=ok\n  guest: SETUP_geekbench=SKIPPED (policy)\nnote BUILD_b=FAIL inside a sentence\n"
                "SETUP_c=FAILED\n  guest: SETUP_d=FAIL rc=1\nBUILD_e=FAIL\n"
            )
            self.assertEqual(provision_failures(log), ["SETUP_d", "BUILD_e"])


def load_gate():
    import sys

    sys.path.insert(0, str(ROOT / "scripts"))
    try:
        from kfci import apps_gate
        from kfci.step import Context
    finally:
        sys.path.pop(0)
    return apps_gate, Context


class ProvisioningBounds(unittest.TestCase):
    """The provisioning is capped in TOTAL and needs free disk; the reserve is derived from the cap."""

    gate, Context = load_gate()

    def test_the_reserve_is_the_provisioning_cap_plus_the_matrix_cap_plus_five_minutes(self):
        m = apps.payload()
        derived = self.gate.PROVISION_CAP_S + self.gate.MATRIX_BUDGET_DEFAULT_S + 5 * 60
        self.assertEqual(m.APPS_MIN_REMAINING_MS, derived * 1000)

    def test_the_cap_is_forty_five_minutes_at_most(self):
        self.assertLessEqual(self.gate.PROVISION_CAP_S, 45 * 60)

    def test_the_cap_leaves_slack_over_the_measured_provisioning(self):
        self.assertGreaterEqual(self.gate.PROVISION_CAP_S, (29 + 10) * 60)

    def test_the_matrix_cap_covers_the_measured_matrix(self):
        self.assertGreaterEqual(self.gate.MATRIX_BUDGET_DEFAULT_S, 1029)

    def test_the_default_minimum_free_space_is_sixty_gigabytes(self):
        self.assertEqual(self.gate.APPS_MIN_FREE_GB, 60)

    def run_gate(self, build="", guest="", patches=(), free=None, env=None, preimage=False):
        """The stage in this process, with fake build scripts, under patched constants; returns (rc, output, files, secs)."""
        import io
        import time

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = build_tree(root)
            (scripts / "apps/build_bundle.sh").write_text(
                "#!/bin/bash\ntouch \"$KF_APPS_ROOT/build.ran\"\n" + (build or StageProvisioning.GOOD_BUILD)
            )
            (scripts / "apps/provision_guest_apps.sh").write_text(
                "#!/bin/bash\ntouch \"$KF_APPS_ROOT/guest.ran\"\n" + (guest or StageProvisioning.GOOD_GUEST)
            )
            bench = root / "bench"
            bench.mkdir()
            (bench / "guest.qcow2").write_text("image")
            (bench / "guest.qcow2.STAGED").write_text("image=/x\n")
            if preimage:  # the guest image copy already exists, so the stage goes straight to the guest image step
                (bench / "guest_apps.qcow2").write_text("image")
            (root / "apps").mkdir()
            (root / "list.txt").write_text("vectorAdd\ndeviceQuery\n")
            (root / "matrix.sh").write_text(
                "#!/bin/bash\nR=%s/apps/results/$2; mkdir -p $R\ntouch %s/matrix.ran\n" % (root, root)
                + "".join(
                    "echo 'APPRES side=guest app=%s verdict=PASS rc=0 secs=2 quiet=0 note=- boot=b1 guest_xid=0 kf3_lines=1 kf3_refusals=0' >> $R/guest.res\n"
                    % name
                    for name in ("vectorAdd", "deviceQuery")
                )
            )
            environment = {
                "PATH": os.environ["PATH"],
                "KF_APPS_LIST": str(root / "list.txt"),
                "KF_APPS_ROOT": str(root / "apps"),
                "KF_APPS_MATRIX": str(root / "matrix.sh"),
                "KF_APPS_BUDGET_S": "30",
                "KF_APPS_POLL_S": "0.2",
                "BENCH_DIR": str(bench),
                "KF_APPS_PROVISION": "1",
                "KF_APPS_TEST_MIN_FREE_GB": "0",
            }
            environment.update(env or {})
            out = io.StringIO()
            ctx = self.Context(
                dict(tag="ci0abc", prov=str(root / "prov"), which="middle"),
                environment, out, time.monotonic() + 600, root=root,
            )
            started = time.monotonic()
            with contextlib.ExitStack() as stack:
                for name, value in patches:
                    stack.enter_context(patch.object(self.gate, name, value))
                if free is not None:
                    stack.enter_context(patch.object(self.gate, "free_bytes", side_effect=free))
                result = self.gate.AppsGate().run(ctx)
            files = {n: (root / "apps" / n).exists() for n in ("build.ran", "guest.ran", "PROVISIONED")}
            files["matrix.ran"] = (root / "matrix.ran").exists()
            pidfile = bench / "appsprov.pid"
            files["pid"] = int(pidfile.read_text()) if pidfile.exists() else None
            files["image"] = (bench / "guest_apps.qcow2").exists()
            return result.rc, out.getvalue(), files, time.monotonic() - started

    # ---- the total cap -------------------------------------------------------------------------------------------
    def test_a_hanging_build_is_stopped_at_the_cap_and_is_a_media_retry(self):
        rc, out, files, secs = self.run_gate(build="sleep 60\n", patches=[("PROVISION_CAP_S", 2)])
        self.assertEqual(rc, 3, out)
        self.assertIn("APPS_STAGE_RETRY class=MEDIA", out)
        self.assertIn("APPS_PROVISION_END rc=124", out)
        self.assertLess(secs, 30)

    def test_a_hanging_build_never_reaches_the_guest_image(self):
        rc, out, files, secs = self.run_gate(build="sleep 60\n", patches=[("PROVISION_CAP_S", 2)])
        self.assertFalse(files["guest.ran"])
        self.assertFalse(files["matrix.ran"])
        self.assertFalse(files["PROVISIONED"])
        self.assertLess(secs, 30)  # stopped at the cap, not after the build's own 60 s

    def test_a_hanging_guest_image_is_stopped_at_the_cap_and_is_a_media_retry(self):
        rc, out, files, secs = self.run_gate(guest="sleep 60\n", patches=[("PROVISION_CAP_S", 3)])
        self.assertEqual(rc, 3, out)
        self.assertIn("APPS_STAGE_RETRY class=MEDIA", out)
        self.assertLess(secs, 30)
        self.assertFalse(files["PROVISIONED"])
        self.assertFalse(files["matrix.ran"])

    def test_the_cap_is_shared_by_the_build_and_the_guest_image(self):
        # the build uses 3 s of a 5 s cap; the guest image gets only the 2 s that are left, not a fresh cap
        rc, out, files, secs = self.run_gate(
            build=StageProvisioning.GOOD_BUILD + "sleep 3\n", guest="sleep 60\n", patches=[("PROVISION_CAP_S", 5)]
        )
        self.assertEqual(rc, 3, out)
        self.assertLess(secs, 6.5)  # 3 s of build plus the 2 s that were left, not plus a fresh 5 s
        self.assertFalse(files["matrix.ran"])

    def test_a_build_that_used_the_whole_cap_does_not_start_the_guest_image(self):
        rc, out, files, secs = self.run_gate(
            build=StageProvisioning.GOOD_BUILD + "sleep 4.2\n", patches=[("PROVISION_CAP_S", 5)], preimage=True
        )
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["guest.ran"])

    def test_the_guest_image_gets_only_the_time_the_build_left(self):
        rc, out, files, secs = self.run_gate(
            build=StageProvisioning.GOOD_BUILD + "sleep 2\n", guest="sleep 60\n", patches=[("PROVISION_CAP_S", 4)]
        )
        self.assertEqual(rc, 3, out)
        self.assertLess(secs, 5.5)

    def test_a_build_that_left_under_a_second_does_not_start_the_guest_image(self):
        rc, out, files, secs = self.run_gate(
            build=StageProvisioning.GOOD_BUILD + "sleep 4.6\n", patches=[("PROVISION_CAP_S", 5)], preimage=True
        )
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["guest.ran"])

    def test_provisioning_inside_the_cap_still_passes(self):
        rc, out, files, secs = self.run_gate(patches=[("PROVISION_CAP_S", 60)])
        self.assertEqual(rc, 0, out)
        self.assertTrue(files["PROVISIONED"])

    # ---- the free-disk guard -------------------------------------------------------------------------------------
    def test_low_free_disk_is_a_media_retry_with_the_numbers_and_nothing_is_built(self):
        rc, out, files, _ = self.run_gate(free=lambda path: 10 * 10**9, env={"KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertEqual(rc, 3, out)
        self.assertIn("APPS_PROVISION_DISK free_gb=10 need_gb=60", out)
        self.assertIn("APPS_STAGE_RETRY class=MEDIA", out)
        self.assertFalse(files["build.ran"])
        self.assertFalse(files["matrix.ran"])

    def test_just_under_the_minimum_is_refused_and_the_minimum_is_accepted(self):
        rc, out, files, _ = self.run_gate(free=lambda path: 60 * 10**9 - 1, env={"KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["build.ran"])
        rc, out, files, _ = self.run_gate(free=lambda path: 60 * 10**9, env={"KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertEqual(rc, 0, out)
        self.assertTrue(files["build.ran"])

    def test_the_smaller_of_the_two_file_systems_decides(self):
        def free(path):
            return 500 * 10**9 if str(path).endswith("/apps") else 20 * 10**9

        rc, out, files, _ = self.run_gate(free=free, env={"KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertEqual(rc, 3, out)
        self.assertIn("free_gb=20 need_gb=60", out)
        self.assertFalse(files["build.ran"])

    def test_a_small_apps_file_system_decides_and_the_row_names_its_free_space(self):
        def free(path):
            return 20 * 10**9 if str(path).endswith("/apps") else 500 * 10**9

        rc, out, files, _ = self.run_gate(free=free, env={"KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertEqual(rc, 3, out)
        self.assertIn("free_gb=20 need_gb=60", out)
        self.assertFalse(files["build.ran"])

    def test_the_bench_file_system_just_under_the_minimum_decides(self):
        def free(path):
            return 500 * 10**9 if str(path).endswith("/apps") else 60 * 10**9 - 1

        rc, out, files, _ = self.run_gate(free=free, env={"KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["build.ran"])

    def test_the_apps_file_system_just_under_the_minimum_decides(self):
        def free(path):
            return 60 * 10**9 - 1 if str(path).endswith("/apps") else 500 * 10**9

        rc, out, files, _ = self.run_gate(free=free, env={"KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["build.ran"])

    def test_the_guard_does_not_apply_without_provisioning(self):
        rc, out, files, _ = self.run_gate(free=lambda path: 0, env={"KF_APPS_PROVISION": "0", "KF_APPS_TEST_MIN_FREE_GB": ""})
        self.assertNotIn("APPS_PROVISION_DISK", out)
        self.assertFalse(files["build.ran"])

    def test_free_bytes_uses_the_nearest_existing_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertGreater(self.gate.free_bytes(Path(directory) / "no/such/dir"), 0)


def alive(pid):
    """True while the process runs; a zombie that nobody reaped counts as dead."""
    try:
        os.kill(pid, 0)
        state = Path("/proc/%d/stat" % pid).read_text().split(") ")[-1][:1]
    except (ProcessLookupError, OSError):
        return False
    return state != "Z"


class StageKnobs(unittest.TestCase):
    """The stage's knobs cannot be set from the environment in production; the tests have one explicit seam."""

    gate, Context = load_gate()
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture
    go = PayloadPlansProvisioning.go

    def run_gate(self, **kw):
        return ProvisioningBounds.run_gate(self, **kw)

    def test_the_payload_pops_every_knob_before_any_stage(self):
        snapshots = []
        env = {k: "1" for k in ("KF_APPS_BUDGET_S", "KF_APPS_MIN_FREE_GB", "KF_APPS_TEST_MIN_FREE_GB")}
        rc, _, _ = self.go(env=env, probe=snapshots.append)
        self.assertEqual(rc, 0)
        self.assertTrue(snapshots)
        for knob in env:
            self.assertTrue(all(knob not in snap for snap in snapshots), knob)

    def test_the_budget_knob_is_popped_by_the_direct_call(self):
        env = {"KF_LINUX_APPS": "1", "KF_APPS_BUDGET_S": "99999"}
        apps.payload().linuxapps_requested(env, "middle")
        self.assertNotIn("KF_APPS_BUDGET_S", env)

    def test_the_free_space_knobs_are_popped_by_the_direct_call(self):
        env = {"KF_LINUX_APPS": "1", "KF_APPS_MIN_FREE_GB": "0", "KF_APPS_TEST_MIN_FREE_GB": "0"}
        apps.payload().linuxapps_requested(env, "middle")
        self.assertEqual(env, {})

    def test_none_of_the_knobs_has_a_row_in_the_environment_contract(self):
        text = (HERE / "vast_report.py").read_text()
        for knob in ("KF_APPS_BUDGET_S", "KF_APPS_MIN_FREE_GB", "KF_APPS_TEST_MIN_FREE_GB", "KF_APPS_PROVISION"):
            self.assertNotIn(knob, text)

    # ---- the matrix budget is clamped to the default --------------------------------------------------------------
    def test_a_longer_budget_is_cut_to_the_default(self):
        for value in ("99999", "3001", "51m", "2h", "1d"):
            self.assertEqual(self.gate.clamp_budget(value), str(self.gate.MATRIX_BUDGET_DEFAULT_S), value)

    def test_a_shorter_or_equal_budget_is_kept(self):
        for value in ("30", "3000", "10m", "50m", "1s"):
            self.assertEqual(self.gate.clamp_budget(value), value)

    def test_no_budget_is_the_default_and_zero_is_the_default(self):
        self.assertEqual(self.gate.clamp_budget(None), "3000")
        self.assertEqual(self.gate.clamp_budget(""), "3000")
        self.assertEqual(self.gate.clamp_budget("0"), "3000")

    def test_an_unparsable_budget_is_passed_on_to_fail_there(self):
        self.assertEqual(self.gate.clamp_budget("abc"), "abc")

    def test_the_stage_uses_the_clamped_budget(self):
        rc, out, files, _ = self.run_gate(env={"KF_APPS_BUDGET_S": "99999"})
        self.assertIn("budget_s=3000 ", out)

    # ---- the free-space minimum ----------------------------------------------------------------------------------
    def test_the_old_knob_name_no_longer_lowers_the_minimum(self):
        rc, out, files, _ = self.run_gate(
            free=lambda path: 10 * 10**9, env={"KF_APPS_TEST_MIN_FREE_GB": "", "KF_APPS_MIN_FREE_GB": "0"}
        )
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["build.ran"])

    def test_a_negative_minimum_is_an_error_not_a_disabled_guard(self):
        rc, out, files, _ = self.run_gate(free=lambda path: 10 * 10**9, env={"KF_APPS_TEST_MIN_FREE_GB": "-5"})
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["build.ran"])

    def test_another_negative_minimum_is_an_error_too(self):
        rc, out, files, _ = self.run_gate(free=lambda path: 10**12, env={"KF_APPS_TEST_MIN_FREE_GB": "-1"})
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["build.ran"])

    def test_a_garbage_minimum_is_an_error_not_a_disabled_guard(self):
        rc, out, files, _ = self.run_gate(free=lambda path: 10**12, env={"KF_APPS_TEST_MIN_FREE_GB": "lots"})
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["build.ran"])


class ExpectedRows(unittest.TestCase):
    """A missing row is a provisioning failure, like a FAIL row (scripts/apps/expected_rows.txt, the one list)."""

    run_gate = ProvisioningBounds.run_gate
    gate, Context = load_gate()

    def test_the_list_is_exactly_what_the_stage_expects(self):
        self.assertEqual(
            EXPECTED_BUILD,
            ["BUILD_" + n for n in (
                "open_dev_packages", "cuda_libs", "cuda_headers", "src_cuda_samples", "src_gpu_burn",
                "src_llama_cpp", "src_clpeak", "egl_offscreen", "gpu_burn", "llama_cpp", "clpeak", "vkpeak",
                "bundle_tar")],
        )
        self.assertEqual(
            EXPECTED_SETUP,
            ["SETUP_" + n for n in ("apt", "icd", "torch", "pylibs", "imports", "hf_model", "gguf", "blender")],
        )

    def test_every_expected_row_is_printed_by_its_script(self):
        bundle = (ROOT / "scripts/apps/build_bundle.sh").read_text()
        guest = (ROOT / "scripts/apps/setup_side.sh").read_text()
        for row in EXPECTED_BUILD:
            self.assertRegex(bundle, r"\bok %s \$" % row[len("BUILD_"):], row)
        for row in EXPECTED_SETUP:
            self.assertRegex(guest, r"\bok %s \$" % row[len("SETUP_"):], row)

    GUEST_LOOP = re.compile(r"while read -r ROW; do\n.*?\ndone < <\(grep -E '\^SETUP_' \"\$HERE/expected_rows.txt\"\)\n", re.S)

    def check_loop(self, setup_out):
        """Run the expected-rows loop of provision_guest_apps.sh on a given guest output; returns SETUP_FAILS."""
        text = (ROOT / "scripts/apps/provision_guest_apps.sh").read_text()
        loop = self.GUEST_LOOP.search(text)
        self.assertIsNotNone(loop)
        script = 'say() { echo "$*"; }\nHERE="%s"\nSETUP_FAILS=0\nSETUP_OUT=$(cat)\n%secho "FAILS=$SETUP_FAILS"\n' % (
            ROOT / "scripts/apps", loop[0])
        got = subprocess.run(["bash", "-c", script], input=setup_out, capture_output=True, text=True, timeout=30)
        return int(re.search(r"FAILS=(\d+)", got.stdout)[1])

    def guest_output(self, drop=None, replace=None):
        rows = ["  guest: %s=ok" % r for r in EXPECTED_SETUP if r != drop]
        text = "\n".join(rows) + "\n  guest: SETUP_SIDE_DONE\n"
        return text.replace(*replace) if replace else text

    def test_the_guest_script_counts_nothing_when_every_row_is_ok(self):
        self.assertEqual(self.check_loop(self.guest_output()), 0)

    def test_the_guest_script_counts_every_missing_row(self):
        for row in EXPECTED_SETUP:
            with self.subTest(row):
                self.assertEqual(self.check_loop(self.guest_output(drop=row)), 1)

    def test_the_guest_script_counts_a_row_that_is_not_ok(self):
        self.assertEqual(self.check_loop(self.guest_output(replace=("SETUP_gguf=ok", "SETUP_gguf=SKIPPED"))), 1)

    def test_the_guest_script_still_exits_five_on_any_count(self):
        text = (ROOT / "scripts/apps/provision_guest_apps.sh").read_text()
        self.assertIn('[ "$SETUP_FAILS" -eq 0 ] || {', text)
        self.assertIn("exit 5", text)

    def test_every_build_row_missing_is_a_media_retry(self):
        for row in EXPECTED_BUILD:
            with self.subTest(row):
                build = StageProvisioning.GOOD_BUILD.replace("echo %s=ok\n" % row, "")
                self.assertNotEqual(build, StageProvisioning.GOOD_BUILD)
                rc, out, files, _ = self.run_gate(build=build)
                self.assertEqual(rc, 3, out)
                self.assertIn("APPS_STAGE_RETRY class=MEDIA", out)
                self.assertIn("missing_" + row, out)
                self.assertFalse(files["guest.ran"])
                self.assertFalse(files["matrix.ran"])

    def test_every_setup_row_missing_is_a_media_retry(self):
        for row in EXPECTED_SETUP:
            with self.subTest(row):
                guest = StageProvisioning.GOOD_GUEST.replace("echo '  guest: %s=ok'\n" % row, "")
                self.assertNotEqual(guest, StageProvisioning.GOOD_GUEST)
                rc, out, files, _ = self.run_gate(guest=guest)
                self.assertEqual(rc, 3, out)
                self.assertIn("APPS_STAGE_RETRY class=MEDIA", out)
                self.assertIn("missing_" + row, out)
                self.assertFalse(files["PROVISIONED"])
                self.assertFalse(files["matrix.ran"])

    def test_a_row_that_is_not_ok_does_not_count_as_present(self):
        rc, out, files, _ = self.run_gate(guest=StageProvisioning.GOOD_GUEST.replace("SETUP_gguf=ok", "SETUP_gguf=SKIPPED"))
        self.assertEqual(rc, 3, out)
        self.assertIn("missing_SETUP_gguf", out)

    def test_all_rows_present_is_not_a_failure(self):
        rc, out, files, _ = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertNotIn("APPS_PROVISION_FAIL", out)


class BoundedCopyAndQemu(unittest.TestCase):
    """The image copy is inside the shared cap, and a daemonized provisioning QEMU does not outlive a failed stage."""

    run_gate = ProvisioningBounds.run_gate
    gate, Context = load_gate()

    def fake_path(self, name, body):
        directory = Path(tempfile.mkdtemp(prefix="fakebin-"))
        self.addCleanup(shutil.rmtree, directory, True)
        tool = directory / name
        tool.write_text("#!/bin/bash\n" + body)
        tool.chmod(0o755)
        return {"PATH": str(directory) + ":" + os.environ["PATH"]}

    def test_a_hanging_image_copy_is_stopped_at_the_cap(self):
        env = self.fake_path("cp", "sleep 60\n")
        rc, out, files, secs = self.run_gate(env=env, patches=[("PROVISION_CAP_S", 3)])
        self.assertEqual(rc, 3, out)
        self.assertIn("APPS_STAGE_RETRY class=MEDIA", out)
        self.assertLess(secs, 30)

    def test_a_hanging_image_copy_never_reaches_the_guest_image(self):
        env = self.fake_path("cp", "sleep 60\n")
        rc, out, files, secs = self.run_gate(env=env, patches=[("PROVISION_CAP_S", 3)])
        self.assertFalse(files["guest.ran"])
        self.assertFalse(files["PROVISIONED"])
        self.assertLess(secs, 30)

    def test_a_failed_copy_is_a_media_retry_and_leaves_no_image(self):
        env = self.fake_path("cp", 'echo partial > "$3"; exit 1\n')
        rc, out, files, _ = self.run_gate(env=env)
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["image"])
        self.assertFalse(files["guest.ran"])

    def test_a_copy_killed_at_the_cap_leaves_no_image(self):
        env = self.fake_path("cp", 'echo partial > "$3"; sleep 60\n')
        rc, out, files, _ = self.run_gate(env=env, patches=[("PROVISION_CAP_S", 3)])
        self.assertEqual(rc, 3, out)
        self.assertFalse(files["image"])

    def test_the_real_copy_still_works(self):
        rc, out, files, _ = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertTrue(files["image"])

    # A copy of bash under the name of a QEMU (coreutils may be one multicall binary that refuses a renamed copy of sleep);
    # it waits on a fifo with a timeout, no child process.
    FAKE = 'mkfifo "$0.fifo"; read -t 300 < "$0.fifo"'
    QEMU = (
        'cp /bin/bash "$KF_APPS_ROOT/qemu-system-x86_64"\n'
        'setsid "$KF_APPS_ROOT/qemu-system-x86_64" -c \'mkfifo "$0.fifo"; read -t 300 < "$0.fifo"\' "$KF_APPS_ROOT/fake" >/dev/null 2>&1 &\n'
        'echo $! > "$BENCH_DIR/appsprov.pid"\n'
        'sleep 1\n'
    )

    def test_a_daemonized_qemu_is_stopped_when_the_guest_image_times_out(self):
        rc, out, files, _ = self.run_gate(guest=self.QEMU + "sleep 60\n", patches=[("PROVISION_CAP_S", 4)])
        self.assertEqual(rc, 3, out)
        self.assertIsNotNone(files["pid"])
        self.assertFalse(alive(files["pid"]))

    def test_a_daemonized_qemu_is_stopped_when_the_guest_image_fails(self):
        rc, out, files, _ = self.run_gate(guest=self.QEMU + "exit 5\n")
        self.assertEqual(rc, 3, out)
        self.assertIsNotNone(files["pid"])
        self.assertFalse(alive(files["pid"]))

    def stop(self, name):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / name
            shutil.copyfile("/bin/bash", binary)
            binary.chmod(0o755)
            import subprocess

            child = subprocess.Popen([str(binary), "-c", 'mkfifo "$0.fifo"; read -t 30 < "$0.fifo"', str(binary)])
            time.sleep(0.3)
            self.addCleanup(lambda: (child.kill(), child.wait()))
            (Path(directory) / "appsprov.pid").write_text("%d\n" % child.pid)
            result = self.gate.stop_provisioning_qemu(directory)
            return result, child

    def test_only_a_qemu_is_stopped(self):
        result, child = self.stop("sleep")
        self.assertFalse(result)
        self.assertIsNone(child.poll())

    def test_another_process_name_is_not_stopped_either(self):
        result, child = self.stop("notqemu")
        self.assertFalse(result)
        self.assertIsNone(child.poll())

    def test_a_qemu_is_stopped(self):
        result, child = self.stop("qemu-system-x86_64")
        self.assertTrue(result)
        child.wait(timeout=5)

    def test_a_missing_or_garbage_pidfile_is_ignored(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertFalse(self.gate.stop_provisioning_qemu(directory))
            (Path(directory) / "appsprov.pid").write_text("not a pid\n")
            self.assertFalse(self.gate.stop_provisioning_qemu(directory))
            (Path(directory) / "appsprov.pid").write_text("1\n")
            self.assertFalse(self.gate.stop_provisioning_qemu(directory))

    def test_the_stage_timeout_matches_what_the_code_does(self):
        step = self.gate.AppsGate()
        cap, budget = self.gate.PROVISION_CAP_S, self.gate.MATRIX_BUDGET_DEFAULT_S
        self.assertEqual(step.timeout_seconds, cap + budget + 300)

    def test_the_stage_timeout_covers_the_matrix_kill_grace(self):
        step = self.gate.AppsGate()
        self.assertGreaterEqual(step.timeout_seconds, self.gate.PROVISION_CAP_S + self.gate.MATRIX_BUDGET_DEFAULT_S + 30)

    def test_the_stage_timeout_is_not_the_old_two_hours_of_provisioning(self):
        step = self.gate.AppsGate()
        self.assertLessEqual(step.timeout_seconds, self.gate.PROVISION_CAP_S + self.gate.MATRIX_BUDGET_DEFAULT_S + 600)


class WindowsWithoutWindowsApps(unittest.TestCase):
    setUp = PayloadPlansProvisioning.setUp
    _main_fixture = v7.V7Tests._main_fixture
    go = PayloadPlansProvisioning.go

    def minutes(self, extra=0):
        gate = self.m.WINDOWS_MIN_REMAINING_MS
        return (self.m.APPS_MIN_REMAINING_MS + gate) // 60000 + extra

    def test_the_windows_gate_is_reserved_when_there_are_no_windows_apps(self):
        rc, result, calls = self.go(soft=self.minutes(-1), windows="1")
        self.assertEqual(rc, 1)
        self.assertNotIn("apps", calls)
        self.assertIn("apps_retry_time", [row["name"] for row in result["suites"]])

    def test_far_too_little_time_for_the_gate_does_not_start_the_apps(self):
        rc, result, calls = self.go(soft=self.minutes(-25), windows="1")
        self.assertEqual(rc, 1)
        self.assertNotIn("apps", calls)

    def test_with_the_gate_reserved_the_apps_start(self):
        rc, result, calls = self.go(soft=self.minutes(2), windows="1")
        self.assertEqual(rc, 0, result)
        self.assertIn("apps", calls)


if __name__ == "__main__":
    unittest.main()
