"""V7 falsifiers: the PR104 result, immutable source and strict fault boundary."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

from test_vast_payload import MERGE, MARK, PROVISION, SHA, XID, watch


class V7Tests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "payload_v7", Path(__file__).with_name("vast_payload.py")
        )
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def test_only_the_marked_control_may_fault(self):
        self.assertFalse(self.m.xid_ok(watch(XID, 0, 1), 6, "--defer-liveness", MARK))

    def test_executor_control_gets_no_exemption(self):
        self.assertFalse(self.m.xid_ok(watch(XID, 0, 1), 6, "--executor-vas", MARK))

    def test_non_ce_client_is_rejected(self):
        bad = XID.replace("HUBCLIENT_CE0", "HUBCLIENT_GR")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_unknown_client_is_rejected(self):
        bad = XID.replace("HUBCLIENT_CE0", "")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_other_fault_type_is_rejected(self):
        bad = XID.replace("FAULT_PDE", "FAULT_PTE")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_read_fault_is_rejected(self):
        bad = XID.replace("VIRT_WRITE", "VIRT_READ")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_duplicate_marker_cannot_expand_allowance(self):
        text = watch(XID + "\n" + XID.replace("20.1", "20.2"), 0, 2)
        self.assertFalse(self.m.xid_ok(text, 6, "--missing-page-fault", MARK * 2))

    def test_marker_must_have_the_source_constant(self):
        wrong = MARK.replace("a11000000", "a12000000")
        bad = XID.replace("0xa_11000000", "0xa_12000000")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", wrong))

    def test_provision_conflicting_rc_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.provision_rows(PROVISION + "DRIVER_RC=1\n", 0)

    def test_provision_conflicting_build_rc_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.provision_rows(PROVISION + "KF3_RC=1\n", 0)

    def test_merge_conflicting_test_totals_refuse(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(MERGE + "TESTS passed 10 failed 2\n", 0, SHA)

    def test_merge_conflicting_gate_summary_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(MERGE + "V3_GATES_SUMMARY pass=8 fail=1 gate10=PASS\n", 0, SHA)

    def test_merge_prefix_survives_partial_evidence(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(MERGE.split("BARE_SUITE_PASS=")[0], 124, SHA)
        self.assertEqual(
            [r["name"] for r in caught.exception.rows], ["kf_tests", "gates", "kf3_build"]
        )

    def test_provision_head_is_bound_to_sha(self):
        source = Path(self.m.__file__).read_text()
        self.assertIn("prepare_checkout(root, sha)", source)
        self.assertIn("/workspace/kf-result.json", source)
        self.assertNotIn("/workspace/kf-ci-result.json", source)

    def test_result_has_only_the_pr104_payload_keys(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "result.json"
            result = {"verdict": "FAIL", "suites": [self.m.row("tests", 2)]}
            self.m.persist_result(path, result)
            self.assertEqual(json.loads(path.read_text()), result)
            self.assertFalse(path.with_suffix(".tmp").exists())

    def test_timeout_keeps_finished_prefix(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "result.json"
            result = {"verdict": "FAIL", "suites": [self.m.row("provision", 1)]}

            def grade(rc):
                raise self.m.EvidenceError("raw_suite", [self.m.row("kf_tests", 20)])

            stages = [
                {
                    "name": "merge",
                    "command": ["false"],
                    "output": path,
                    "progress": [],
                    "grade": grade,
                }
            ]
            rc = self.m.execute_stages(stages, 1, result, path, runner=lambda *args: 124)
            self.assertEqual(rc, 1)
            rows = json.loads(path.read_text())["suites"]
            self.assertEqual([r["name"] for r in rows], ["provision", "kf_tests", "timeout"])
            self.assertEqual(rows[-1]["fail"], 1)

    def test_first_bad_stage_cancels_later_stages(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "result.json"
            result = {"verdict": "FAIL", "suites": []}

            def grade(rc):
                raise self.m.EvidenceError("raw_suite")

            stages = [
                {
                    "name": "raw",
                    "command": ["false"],
                    "output": path,
                    "progress": [],
                    "grade": grade,
                },
                {
                    "name": "thin",
                    "command": ["true"],
                    "output": path,
                    "progress": [],
                    "grade": lambda rc: [self.m.row("thin", 30)],
                },
            ]
            calls = []
            rc = self.m.execute_stages(
                stages, 1, result, path, runner=lambda *a: calls.append(a) or 1
            )
            self.assertEqual(rc, 1)
            self.assertEqual(len(calls), 1)
            self.assertEqual(result["suites"], [self.m.row("raw_suite", failed=1)])

    def test_source_pin_uses_local_exact_approved_objects(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)

            def git(*args):
                return subprocess.check_output(["git", "-C", d, *args], text=True).strip()

            git("init", "-q")
            git("config", "user.email", "fixture@example.invalid")
            git("config", "user.name", "Fixture")
            git("commit", "-q", "--allow-empty", "-m", "fixture")
            sha = git("rev-parse", "HEAD")
            git("remote", "add", "origin", "https://invalid.example/repository")
            branch = self.m.prepare_checkout(root, sha)
            self.assertEqual(git("remote", "get-url", "origin"), d)
            self.assertEqual(git("rev-parse", "refs/heads/" + branch), sha)
            self.assertEqual(git("rev-parse", "refs/remotes/origin/" + branch), sha)

    def test_wrong_checkout_is_refused_before_remote_changes(self):
        with patch.object(self.m.subprocess, "check_output", return_value="b" * 40):
            with self.assertRaises(self.m.EvidenceError):
                self.m.prepare_checkout(Path("/fixture"), SHA)

    def test_red_cuda_row_is_not_progress(self):
        self.assertTrue(self.m.red_line("CL_ROW mode=host rung=cup3 rep=1 verdict=FAIL rc=1"))

    def test_red_crate_test_totals_are_not_progress(self):
        self.assertTrue(self.m.red_line("TESTS passed 90 failed 2"))

    def test_hook_failure_stops_owned_child(self):
        with tempfile.TemporaryDirectory() as d:
            failure = Path(d) / "xid-failed"
            code = "import time;time.sleep(20)"
            failure.touch()
            with patch.dict(os.environ, {"KF_PAYLOAD_XID_FAILED": str(failure)}):
                started = time.monotonic()
                rc = self.m.run_bounded(
                    ["python3", "-c", code], int(time.time() * 1000) + 30000, Path(d) / "log", []
                )
            self.assertNotEqual(rc, 0)
            self.assertLess(time.monotonic() - started, 3)

    def test_watch_failure_retains_merge_stage_attribution(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "watch"
            log.write_text(watch(rc=1))

            def grade(rc):
                self.assertEqual(rc, 1)
                raise self.m.EvidenceError("raw_suite", [self.m.row("kf_tests", 20)])

            with self.assertRaises(self.m.EvidenceError) as caught:
                self.m.stage_grade(None, None, log, grade, 1)
            self.assertEqual(caught.exception.name, "raw_suite")
            self.assertEqual(caught.exception.rows[0]["pass"], 20)

    def test_watch_failure_retains_provision_attribution(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "watch"
            log.write_text(watch(rc=2))

            def grade(rc):
                self.assertEqual(rc, 2)
                raise self.m.EvidenceError("provision")

            with self.assertRaises(self.m.EvidenceError) as caught:
                self.m.stage_grade(None, None, log, grade, 2)
            self.assertEqual(caught.exception.name, "provision")

    def test_timeout_does_not_erase_completed_tests(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(MERGE.split("GATES_RC=")[0], 124, SHA)
        self.assertEqual(caught.exception.rows, [self.m.row("kf_tests", 2000)])

    def test_timeout_really_stops_grandchild_work(self):
        import sys

        with tempfile.TemporaryDirectory() as d:
            marker = Path(d) / "late"
            child = "import time,pathlib;time.sleep(1);pathlib.Path(" + repr(str(marker))
            child += ").touch()"
            code = 'import subprocess,sys,time;subprocess.Popen([sys.executable,"-c",'
            code += repr(child) + "]);time.sleep(10)"
            rc = self.m.run_bounded(
                [sys.executable, "-c", code], int(time.time() * 1000) + 100, Path(d) / "log", []
            )
            self.assertEqual(rc, 124)
            time.sleep(1.1)
            self.assertFalse(marker.exists())

    def test_result_is_accepted_by_pr104_reader(self):
        spec = importlib.util.spec_from_file_location(
            "report", Path(__file__).with_name("vast_report.py")
        )
        report = importlib.util.module_from_spec(spec)
        import sys

        sys.modules["report"] = report
        spec.loader.exec_module(report)
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "result.json"
            result = {
                "verdict": "FAIL",
                "suites": [self.m.row("raw_suite", failed=1)],
                "hardware": {
                    "gpu_name": "RTX 3060",
                    "compute_cap": 860,
                    "driver_version": "580.159.04",
                },
            }
            self.m.persist_result(path, result)
            self.assertEqual(report.read_result_file(path), result)

    def test_hardware_converts_smi_units(self):
        with patch.object(
            self.m.subprocess,
            "check_output",
            return_value="NVIDIA GeForce RTX 3060, 8.6, 580.159.04\n",
        ):
            self.assertEqual(self.m.hardware()["compute_cap"], 860)

    def test_two_gpus_cannot_masquerade_as_one(self):
        text = "RTX 3060, 8.6, 580.159.04\n" * 2
        with patch.object(self.m.subprocess, "check_output", return_value=text):
            self.assertEqual(self.m.hardware()["compute_cap"], 0)

    def test_malformed_hardware_stays_unknown(self):
        with patch.object(self.m.subprocess, "check_output", return_value="bad"):
            self.assertEqual(self.m.hardware()["gpu_name"], "unknown")

    def _main_fixture(
        self,
        failure=None,
        free=100_000_000_000,
        kvm=True,
        legacy_client=None,
        initial_hardware=None,
        required_profile=None,
    ):
        from types import SimpleNamespace

        native_path = Path
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            repo = root / "repo"
            (repo / "scripts/ci").mkdir(parents=True)
            (repo / "scripts/bench").mkdir(parents=True)
            (repo / "scripts/bench/provision_box.sh").write_text("# fixture")
            (repo / "tools/drivermatrix").mkdir(parents=True)
            (repo / "tools/drivermatrix/tags.txt").write_text("580.159.04\n")
            (root / "root/prov").mkdir(parents=True)
            (root / "workspace/bench").mkdir(parents=True)
            if kvm:
                (root / "dev").mkdir()
                (root / "dev/kvm").touch()

            def mapped(value, *args):
                value = str(value)
                if value.startswith(("/workspace/", "/root/", "/dev/")):
                    return native_path(d, value.lstrip("/"), *args)
                return native_path(value, *args)

            calls = []
            original_bounded_text = self.m.bounded_text

            def runner(command, deadline, output, progress):
                if required_profile:
                    self.assertEqual(os.environ.get("KF_PROVISION_PROFILE"), required_profile)
                self.assertEqual(
                    os.environ["BASH_ENV"], str(repo / "scripts/ci/vast_payload_hooks.sh")
                )
                tag = command[2]
                name = tag.split("_", 1)[1]
                calls.append(name)
                bench = root / "workspace/bench"
                (bench / ("xid_" + tag + ".log")).write_text(watch())
                if name == "provision":
                    (root / "root/prov/prov.log").write_text(PROVISION)
                    if legacy_client:
                        relative = (
                            "x86_64-unknown-linux-musl/release/kayfabe-rm-ladder"
                            if legacy_client == "musl"
                            else "release/kayfabe-rm-ladder"
                        )
                        built = mapped(os.environ["CARGO_TARGET_DIR"]) / relative
                        built.parent.mkdir(parents=True, exist_ok=True)
                        built.touch()
                        # Existing bench/fastguest consumers resolve this exact checkout path.
                        if not (repo / "target" / relative).is_file():
                            return 7
                elif name == "merge_bar":
                    value = MERGE
                    if failure == "merge_bar":
                        value = MERGE.split("BARE_SUITE_PASS=")[0]
                    (root / "root/prov" / (tag.rsplit("_", 2)[0] + ".log")).write_text(value)
                else:
                    mode = "host" if name == "cuda_host" else "guest"
                    from test_vast_payload import ladder

                    original_tag = command[-3]
                    (bench / ("cl_" + original_tag + "_" + mode + ".out")).write_text(ladder(mode))
                return 124 if name == failure else 0

            hardware = {"gpu_name": "RTX 3060", "compute_cap": 860, "driver_version": "580.159.04"}
            with patch.object(
                self.m, "__file__", str(repo / "scripts/ci/vast_payload.py")
            ), patch.object(self.m, "Path", side_effect=mapped), patch.object(
                self.m, "prepare_checkout", return_value="fixture"
            ), patch.object(
                self.m, "hardware", side_effect=[initial_hardware or hardware, hardware]
            ), patch.object(
                self.m, "bounded_text", side_effect=lambda p: original_bounded_text(mapped(p))
            ), patch.object(
                self.m.shutil, "copyfile"
            ), patch.object(
                self.m.shutil, "disk_usage", return_value=SimpleNamespace(free=free)
            ), patch.object(
                self.m.subprocess, "run"
            ), patch.object(
                self.m.execute_stages, "__defaults__", (runner,)
            ), patch.dict(
                os.environ,
                {
                    "KF_SOFT_DEADLINE_MS": str(int(time.time() * 1000) + 10000),
                    "KF_PROVISION_PROFILE": "full",
                },
            ), patch(
                "sys.argv", ["vast_payload.py", SHA, "a" * 32]
            ):
                rc = self.m.main()
            result = json.loads((root / "workspace/kf-result.json").read_text())
            return rc, result, calls

    def test_whole_adapter_uses_existing_flows_in_order(self):
        rc, result, calls = self._main_fixture()
        self.assertEqual(rc, 0)
        self.assertEqual(calls, ["provision", "cuda_host", "merge_bar", "cuda_guest"])
        self.assertEqual(result["verdict"], "PASS")
        self.assertEqual(len(result["suites"]), 9)
        self.assertEqual(set(result), {"verdict", "suites", "hardware"})

    def test_provisioned_musl_binary_is_found_by_existing_guest_helper(self):
        rc, result, calls = self._main_fixture(legacy_client="musl")
        self.assertEqual(rc, 0)
        self.assertEqual(result["verdict"], "PASS")
        self.assertEqual(calls[-1], "cuda_guest")

    def test_host_binary_is_found_by_existing_fast_guest_consumer(self):
        rc, result, calls = self._main_fixture(legacy_client="host")
        self.assertEqual(rc, 0)
        self.assertEqual(result["verdict"], "PASS")
        self.assertIn("merge_bar", calls)

    def test_failed_provision_reports_the_driver_after_swap(self):
        initial = {"gpu_name": "RTX 3060", "compute_cap": 860, "driver_version": "575.51.03"}
        rc, result, _ = self._main_fixture(failure="provision", initial_hardware=initial)
        self.assertEqual(rc, 1)
        self.assertEqual(result["hardware"]["driver_version"], "580.159.04")

    def test_failed_provision_refreshes_an_initial_unknown_compute_cap(self):
        initial = {"gpu_name": "RTX 3060", "compute_cap": 0, "driver_version": "unknown"}
        rc, result, _ = self._main_fixture(failure="provision", initial_hardware=initial)
        self.assertEqual(rc, 1)
        self.assertEqual(result["hardware"]["compute_cap"], 860)

    def test_payload_forces_minimal_profile_for_all_stages(self):
        rc, result, _ = self._main_fixture(required_profile="ci")
        self.assertEqual(rc, 0)
        self.assertEqual(result["verdict"], "PASS")

    def test_minimal_profile_also_applies_before_failed_provision(self):
        rc, result, _ = self._main_fixture(failure="provision", required_profile="ci")
        self.assertEqual(rc, 1)
        self.assertEqual(result["verdict"], "FAIL")

    def test_whole_adapter_timeout_keeps_merge_prefix(self):
        rc, result, calls = self._main_fixture(failure="merge_bar")
        self.assertEqual(rc, 1)
        self.assertEqual(calls, ["provision", "cuda_host", "merge_bar"])
        self.assertEqual(
            [r["name"] for r in result["suites"]],
            ["provision", "cuda_host", "kf_tests", "gates", "kf3_build", "timeout"],
        )
        self.assertEqual(result["verdict"], "FAIL")

    def test_whole_adapter_disk_preflight_prevents_spend_work(self):
        rc, result, calls = self._main_fixture(free=100)
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])
        self.assertEqual(result["suites"], [self.m.row("preflight", failed=1)])

    def test_whole_adapter_requires_actual_kvm_device(self):
        rc, result, calls = self._main_fixture(kvm=False)
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])
        self.assertEqual(result["suites"], [self.m.row("preflight", failed=1)])

    def _hook_fixture(self, arm, xid):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            bins = root / "bin"
            bins.mkdir()
            kernel = root / "kernel"
            kernel.write_text("fixture kernel one\nfixture kernel two\n")
            scripts = {
                "dmesg": 'cat "$KF_FIXTURE_KERNEL"',
                "nvidia-smi": 'echo "RTX 3060,580.159.04"',
                "sleep": "exit 0",
                "kayfabe-rm-ladder": "printf '%s\\n' '"
                + MARK.strip()
                + "'; "
                + "printf '%s\\n' '"
                + xid
                + '\' >> "$KF_FIXTURE_KERNEL"',
            }
            for name, body in scripts.items():
                path = bins / name
                path.write_text("#!/bin/bash\n" + body + "\n")
                path.chmod(0o700)
            env = {
                **os.environ,
                "PATH": str(bins) + ":" + os.environ["PATH"],
                "KF_FIXTURE_KERNEL": str(kernel),
                "BENCH_DIR": str(root),
                "XID_OUTDIR": str(root),
                "KF_PAYLOAD_ROOT": str(Path(self.m.__file__).parents[2]),
                "KF_PAYLOAD_XID_ALLOWED": str(root / "allowed"),
                "KF_PAYLOAD_XID_FAILED": str(root / "failed"),
                "BASH_ENV": str(Path(self.m.__file__).with_name("vast_payload_hooks.sh")),
            }
            p = subprocess.run(
                [
                    "bash",
                    "-c",
                    'timeout --kill-after=5 5 "$1" "$2"',
                    "fixture",
                    str(bins / "kayfabe-rm-ladder"),
                    arm,
                ],
                env=env,
                capture_output=True,
                text=True,
                timeout=10,
            )
            return p.returncode, (root / "failed").exists(), (root / "allowed").exists()

    def test_real_hook_accepts_only_marked_raw_fault(self):
        self.assertEqual(self._hook_fixture("--missing-page-fault", XID), (0, False, True))

    def test_real_hook_refuses_other_raw_arm(self):
        rc, failed, allowed = self._hook_fixture("--timer", XID)
        self.assertNotEqual(rc, 0)
        self.assertTrue(failed)
        self.assertFalse(allowed)

    def test_real_hook_refuses_non_ce_fault(self):
        rc, failed, allowed = self._hook_fixture(
            "--missing-page-fault", XID.replace("HUBCLIENT_CE0", "HUBCLIENT_GR")
        )
        self.assertNotEqual(rc, 0)
        self.assertTrue(failed)
        self.assertFalse(allowed)

    def test_other_page_fault_type_is_rejected_on_ce1(self):
        bad = XID.replace("FAULT_PDE", "FAULT_PTE").replace("HUBCLIENT_CE0", "HUBCLIENT_CE1")
        self.assertFalse(self.m.xid_ok(watch(bad, 0, 1), 6, "--missing-page-fault", MARK))

    def test_ladder_repeated_last_rung_refuses(self):
        from test_vast_payload import ladder

        bad = ladder().replace("rung=cup8bench ", "rung=cup2 ")
        with self.assertRaises(self.m.EvidenceError):
            self.m.ladder_rows(bad, 0, "host", SHA)

    def test_merge_requires_exit_marker(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(MERGE.split("EXIT rc=")[0], 0, SHA)

    def test_merge_nonzero_recorded_exit_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(MERGE.replace("EXIT rc=0", "EXIT rc=1"), 0, SHA)

    def test_hook_failure_does_not_run_later_marker(self):
        import sys

        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            marker = root / "late"
            failure = root / "xid-failed"
            failure.touch()
            code = "import time,pathlib;time.sleep(1);pathlib.Path("
            code += repr(str(marker)) + ").touch()"
            with patch.dict(os.environ, {"KF_PAYLOAD_XID_FAILED": str(failure)}):
                rc = self.m.run_bounded(
                    [sys.executable, "-c", code], int(time.time() * 1000) + 5000, root / "log", []
                )
            self.assertNotEqual(rc, 0)
            self.assertFalse(marker.exists())

    def test_stage_refuses_unattributed_new_xid(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            log = root / "watch"
            log.write_text(watch(XID, 0, 1))
            (root / "allowed").write_text("")
            with patch.dict(os.environ, {"KF_PAYLOAD_XID_ALLOWED": str(root / "allowed")}):
                with self.assertRaises(self.m.EvidenceError):
                    self.m.stage_grade(None, None, log, lambda rc: [self.m.row("stage", 1)], 6)

    def test_stage_refuses_extra_record_next_to_approved_fault(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            log = root / "watch"
            log.write_text(watch(XID + "\n" + XID.replace("20.1", "20.2"), 0, 2))
            (root / "allowed").write_text(json.dumps(XID) + "\n")
            with patch.dict(os.environ, {"KF_PAYLOAD_XID_ALLOWED": str(root / "allowed")}):
                with self.assertRaises(self.m.EvidenceError):
                    self.m.stage_grade(None, None, log, lambda rc: [self.m.row("stage", 1)], 6)

    def test_raw_red_keeps_earlier_merge_rows(self):
        text = MERGE.split("BARE_SUITE_PASS=")[0]
        text += "BARE_CELL_ARM arm=timer verdict=FAIL rc=1\n"
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(text, 1, SHA)
        self.assertEqual(
            [r["name"] for r in caught.exception.rows], ["kf_tests", "gates", "kf3_build"]
        )

    def test_thin_red_keeps_raw_and_build_rows(self):
        text = MERGE.split("FAST_SUITE_PASS=")[0]
        text += "FAST_CELL_ARM arm=timer verdict=FAIL\n"
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(text, 1, SHA)
        self.assertEqual(
            [r["name"] for r in caught.exception.rows],
            ["kf_tests", "gates", "kf3_build", "raw_suite"],
        )


if __name__ == "__main__":
    unittest.main()
