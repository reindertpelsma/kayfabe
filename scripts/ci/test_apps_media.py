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

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import test_vast_payload_apps as apps
import test_vast_payload_v7 as v7

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
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

    GOOD_BUILD = "echo BUILD_gpu_burn=ok\nmkdir -p \"$KF_APPS_ROOT\"; echo bundle > \"$KF_APPS_ROOT/bundle.tgz\"\n"
    GOOD_GUEST = "echo '  guest: SETUP_gguf=ok'\necho '  guest: SETUP_SIDE_DONE'\n"

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


if __name__ == "__main__":
    unittest.main()
