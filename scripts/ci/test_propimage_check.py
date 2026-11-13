#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Hand-authored fixtures; no VM, network, third-party code or credentials."""

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SRC = Path(__file__).resolve().parents[2] / "scripts/bench/windows/propimage_check.py"
spec = importlib.util.spec_from_file_location("fixture_propimage_check", SRC)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

STEP = b"ISO-10303-21;\nHEADER;ENDSEC;\nDATA;#1=CARTESIAN_POINT('',(0.,0.,0.));ENDSEC;\nEND-ISO-10303-21;"


class T(unittest.TestCase):
    def fixture(self, **changes):
        r = {
            "app": "solidworks",
            "run_id": "a" * 32,
            "started": 100,
            "ended": 120,
            "kind": "FILE",
            "artifact": "model.step",
            "modified": 110,
            "bytes": len(STEP),
            "api_ok": True,
        }
        r.update(changes)
        return r

    def grade(self, record=None, content=None):
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            raw = content if content is not None else STEP
            (d / "model.step").write_bytes(raw)
            r = self.fixture(bytes=len(raw)) if record is None else record
            (d / "result.json").write_text(json.dumps(r))
            return m.verdict("solidworks", "a" * 32, d)

    def test_valid_step_file(self):
        self.assertTrue(self.grade()["ok"])

    def test_missing_end_is_not_a_result(self):
        self.assertFalse(self.grade(content=b"ISO-10303-21; DATA;")["ok"])

    def test_stale_file(self):
        self.assertFalse(self.grade(self.fixture(modified=99))["ok"])

    def test_equal_start_is_stale(self):
        self.assertFalse(self.grade(self.fixture(modified=100))["ok"])

    def test_wrong_run(self):
        self.assertFalse(self.grade(self.fixture(run_id="b" * 32))["ok"])

    def test_replayed_record(self):
        self.assertFalse(self.grade(self.fixture(run_id="old"))["ok"])

    def test_wrong_app(self):
        self.assertFalse(self.grade(self.fixture(app="resolve"))["ok"])

    def test_missing_artifact(self):
        self.assertFalse(self.grade(self.fixture(artifact="absent.step"))["ok"])

    def test_traversal(self):
        self.assertFalse(self.grade(self.fixture(artifact="../model.step"))["ok"])

    def test_absolute_path(self):
        self.assertFalse(self.grade(self.fixture(artifact="/tmp/file"))["ok"])

    def test_bytes_mismatch(self):
        self.assertFalse(self.grade(self.fixture(bytes=999))["ok"])

    def test_empty_file(self):
        self.assertFalse(self.grade(content=b"")["ok"])

    def test_api_failure(self):
        self.assertFalse(self.grade(self.fixture(api_ok=False))["ok"])

    def test_second_api_failure(self):
        self.assertFalse(self.grade(self.fixture(api_ok=0))["ok"])

    def test_rejects_bad_step_header(self):
        self.assertFalse(self.grade(content=STEP.replace(b"ISO-10303-21;", b"NOT-STEP!!!!;"))["ok"])

    def test_second_stale_file(self):
        self.assertFalse(self.grade(self.fixture(modified=50))["ok"])

    def test_future_file(self):
        self.assertFalse(self.grade(self.fixture(modified=121))["ok"])

    def test_uia_existing_running_is_not_fresh(self):
        self.assertFalse(
            m.validate_transition(
                {"before": "Running", "progress": "Running", "after": "Finished"},
                "Running",
                "Finished",
            )
        )

    def test_second_length_mismatch(self):
        self.assertFalse(self.grade(self.fixture(bytes=1))["ok"])

    def test_wrong_app_second(self):
        self.assertFalse(self.grade(self.fixture(app="edge"))["ok"])

    def test_api_string_is_not_true(self):
        self.assertFalse(self.grade(self.fixture(api_ok="true"))["ok"])

    def test_probe_is_not_pass(self):
        self.assertFalse(self.grade(self.fixture(kind="PROBE", ok=True))["ok"])

    def test_window_is_not_work(self):
        self.assertFalse(self.grade(self.fixture(kind="LAUNCH", ok=True))["ok"])

    def test_uia_requires_progress(self):
        self.assertFalse(
            m.validate_transition(
                {"before": "Ready", "progress": "", "after": "Finished"}, "Running", "Finished"
            )
        )

    def test_uia_rejects_stale_finished(self):
        self.assertFalse(
            m.validate_transition(
                {"before": "Finished", "progress": "Running", "after": "Finished"},
                "Running",
                "Finished",
            )
        )

    def test_uia_transition(self):
        self.assertTrue(
            m.validate_transition(
                {"before": "Ready", "progress": "Running", "after": "Finished"},
                "Running",
                "Finished",
            )
        )

    def test_second_missing_progress(self):
        self.assertFalse(
            m.validate_transition({"before": "Ready", "after": "Finished"}, "Running", "Finished")
        )

    def test_uia_wrong_completion(self):
        self.assertFalse(
            m.validate_transition(
                {"before": "Ready", "progress": "Running", "after": "Failed"}, "Running", "Finished"
            )
        )

    def test_every_check_renders_and_writes_its_json(self):
        for n in m.CHECKS:
            s = m.render_check(n, run_id="a" * 32)
            self.assertIn("result.json", s)
            self.assertIn("UIAutomationClient", s)
            self.assertNotIn("SetCursorPos", s)
            self.assertNotIn("_CLICKS", s)

    def test_calibration_vars_are_validated(self):
        for env in ({"X": "1; calc"}, {"bad": "1"}, {"SP_CLICKS": "1,2"}):
            with self.assertRaises(ValueError):
                m.env_prefix(env)

    def test_no_check_signs_in_or_types_credentials(self):
        for n in m.CHECKS:
            self.assertNotRegex(
                m.render_check(n), r"(?i)password|sendkeys|Invoke-WebRequest|Invoke-RestMethod"
            )

    def test_task_is_interactive(self):
        self.assertIn("-LogonType Interactive", m.task_wrapper("x"))

    def test_bad_run_id(self):
        with self.assertRaises(ValueError):
            m.render_check("resolve", run_id="';calc")

    def test_dump_is_bounded_and_scoped(self):
        s = m.render_check("resolve")
        self.assertIn("2048", s)
        self.assertIn("ProcessIdProperty", s)
        self.assertIn("AutomationId", s)

    def test_table_records_evidence(self):
        t = m.table([{"app": "a", "ok": True, "evidence": "FILE", "secs": 3, "signal": "s"}])
        self.assertIn("FILE", t)

    def plan(self):
        return {
            "run": {"id": "run", "name": "Run", "type": "ControlType.Button"},
            "status": {"id": "status", "name": "", "type": "ControlType.Text"},
            "running": "Running",
            "finished": "Finished",
            "timeout": 20,
        }

    def test_uia_plan_renders(self):
        script = m.render_check("superposition", plan=self.plan())
        self.assertIn("$invoke.Invoke()", script)
        self.assertIn("ConvertFrom-Json", script)

    def test_bad_plan(self):
        for delta in ({"timeout": 0}, {"running": "Finished"}, {"run": {}}, {"finished": ""}):
            with self.assertRaises(ValueError):
                m.validate_plan(dict(self.plan(), **delta))

    def test_pulled_files_drive_verdict(self):
        calls = []

        def runner(argv, **kw):
            calls.append((argv, kw))
            if argv[0] == "scp" and argv[-2].endswith("/result.json"):
                Path(argv[-1]).write_text(json.dumps(self.fixture()))
            if argv[0] == "scp" and argv[-2].endswith("/model.step"):
                Path(argv[-1]).write_bytes(STEP)
            body = kw.get("input", "") or ""
            out = "ready" if "{'ready'}" in body else ("bounded" if "{'bounded'}" in body else "")
            return SimpleNamespace(returncode=0, stdout=out, stderr="")

        with (
            tempfile.TemporaryDirectory() as td,
            patch.object(m.uuid, "uuid4", return_value=SimpleNamespace(hex="a" * 32)),
        ):
            row = m.run_check(
                "solidworks",
                {},
                Path(td),
                guest=SimpleNamespace(
                    interactive=lambda *a: runner(["interactive"]),
                    ps=lambda script: runner(["ssh"], input=script),
                    scp=lambda src, dst, to: runner(["scp", src, dst]),
                ),
                sleep=lambda _: None,
            )
            self.assertTrue(row["ok"])
            self.assertEqual(row["evidence"], "FILE")
            self.assertTrue((Path(td) / "solidworks" / ("a" * 32) / "result.json").exists())
        self.assertTrue(
            any(
                "Unregister-ScheduledTask -TaskName kf_app_" + ("a" * 32)
                in (k.get("input", "") or "")
                for _, k in calls
            )
        )

    def test_transfer_failure_does_not_pass(self):
        def runner(argv, **kw):
            return SimpleNamespace(returncode=1, stdout="", stderr="failed")

        with tempfile.TemporaryDirectory() as td:
            row = m.run_check(
                "solidworks",
                {},
                Path(td),
                guest=SimpleNamespace(
                    interactive=lambda *a: runner(["interactive"]),
                    ps=lambda script: runner(["ssh"], input=script),
                ),
            )
            self.assertFalse(row["ok"])

    def test_malformed_result_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            for content in ("{", "[]", "null", '{"ok":true}'):
                (d / "result.json").write_text(content)
                self.assertFalse(m.verdict("solidworks", "a" * 32, d)["ok"])

    def test_uia_saved_observations(self):
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            r = self.fixture(
                app="resolve", kind="UIA", before="Ready", progress="Running", after="Finished"
            )
            (d / "result.json").write_text(json.dumps(r))
            row = m.verdict("resolve", "a" * 32, d, self.plan())
            self.assertTrue(row["ok"])
            self.assertEqual(row["evidence"], "UIA")
            self.assertFalse(m.verdict("resolve", "a" * 32, d)["ok"])

    def test_swtpm_old_rejected(self):
        with self.assertRaisesRegex(ValueError, "swtpm >= 0.10"):
            m.require_swtpm("TPM emulator version 0.6.3")

    def test_swtpm_09_rejected(self):
        with self.assertRaises(ValueError):
            m.require_swtpm("TPM emulator version 0.9.0")

    def test_swtpm_supported(self):
        self.assertEqual(m.require_swtpm("TPM emulator version 0.10.1"), (0, 10, 1))

    def test_swtpm_unknown_rejected(self):
        with self.assertRaises(ValueError):
            m.require_swtpm("unknown")

    def test_gpu_fix_every_instance(self):
        script = m.render_gpu_fix("GPU-test")
        self.assertIn("Get-ChildItem $base", script)
        self.assertIn("EnableGpuFirmware", script)
        self.assertNotIn("0001", script)
        self.assertNotIn("0002", script)
        self.assertIn("nvidia-smi", script)

    def test_gpu_fix_restarts_changed_devices(self):
        script = m.render_gpu_fix("GPU-test")
        self.assertIn("pnputil /restart-device", script)
        self.assertIn("$LASTEXITCODE", script)
        self.assertIn("$expected", script)

    def test_gpu_uuid_wrong_rejected(self):
        self.assertFalse(m.gpu_uuid_matches("GPU-other", "GPU-test"))

    def test_gpu_uuid_missing_rejected(self):
        self.assertFalse(m.gpu_uuid_matches("", "GPU-test"))

    def test_gpu_uuid_matches(self):
        self.assertTrue(m.gpu_uuid_matches("GPU-test\n", "GPU-test"))

    def test_wlms_is_harness(self):
        self.assertEqual(
            m.classify_shutdown([{"event_id": 1074, "process": r"C:\Windows\System32\wlms.exe"}]),
            "harness",
        )

    def test_wlms_case_insensitive(self):
        self.assertEqual(
            m.classify_shutdown([{"event_id": 1074, "process": "WLMS.EXE"}]), "harness"
        )

    def test_other_1074_not_wlms(self):
        self.assertIsNone(m.classify_shutdown([{"event_id": 1074, "process": "shutdown.exe"}]))

    def test_wlms_other_event_not_matched(self):
        self.assertIsNone(m.classify_shutdown([{"event_id": 42, "process": "wlms.exe"}]))

    def test_age_warns_over_55(self):
        self.assertTrue(m.boot_age_guard(3200, 150))

    def test_age_warns_already_over(self):
        self.assertTrue(m.boot_age_guard(3400, 10))

    def test_age_allows_within_budget(self):
        self.assertFalse(m.boot_age_guard(100, 900))

    def test_experiment_off_by_default(self):
        self.assertNotIn("--force-renderer-accessibility", m.render_check("3dmark"))

    def test_experiment_explicit(self):
        self.assertIn(
            "--force-renderer-accessibility",
            m.render_check("3dmark", accessibility_experiment=True),
        )

    def test_gpu_fix_second_instance_loop(self):
        self.assertIn("Get-ChildItem $base", m.render_gpu_fix("GPU-test"))
        self.assertNotIn("Status -ne 'OK'", m.render_gpu_fix("GPU-test"))

    def test_wlms_non1074_again(self):
        self.assertIsNone(m.classify_shutdown([{"event_id": 41, "process": "wlms.exe"}]))

    def test_other_process_1074_again(self):
        self.assertIsNone(m.classify_shutdown([{"event_id": 1074, "process": "winlogon.exe"}]))

    def superposition(self, **changes):
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            log = b"<html>Benchmark running<br/>Benchmark stopped</html>"
            (d / "log.html").write_bytes(log)
            (d / "before.png").write_bytes(b"\x89PNG\r\n\x1a\nbefore")
            (d / "result.png").write_bytes(b"\x89PNG\r\n\x1a\nafter")
            (d / "screen.txt").write_text("Score 7685 FPS Min 15.05 Avg 57.49 Max 69.96")
            record = self.fixture(
                app="superposition", kind="FILE", artifact="log.html", bytes=len(log)
            )
            for key, value in changes.items():
                if key in ("log.html", "before.png", "result.png"):
                    (d / key).write_bytes(value)
                    if key == "log.html":
                        record["bytes"] = len(value)
                elif key == "screen.txt":
                    (d / key).write_text(value)
                else:
                    record[key] = value
            (d / "result.json").write_text(json.dumps(record))
            return m.verdict("superposition", "a" * 32, d)

    def test_superposition_real_work_fixture(self):
        row = self.superposition()
        self.assertTrue(row["ok"])
        self.assertEqual(row["evidence"], "FILE")

    def test_superposition_stale_log(self):
        self.assertFalse(self.superposition(modified=90)["ok"])

    def test_superposition_equal_start_log(self):
        self.assertFalse(self.superposition(modified=100)["ok"])

    def test_superposition_no_result_score(self):
        self.assertFalse(self.superposition(**{"screen.txt": "launcher FPS"})["ok"])

    def test_superposition_no_fps(self):
        self.assertFalse(self.superposition(**{"screen.txt": "Score 7685"})["ok"])

    def test_superposition_unchanged_screen(self):
        self.assertFalse(self.superposition(**{"result.png": b"\x89PNG\r\n\x1a\nbefore"})["ok"])

    def test_superposition_no_completed_interval(self):
        self.assertFalse(self.superposition(**{"log.html": b"Benchmark running"})["ok"])

    def test_superposition_reversed_interval(self):
        self.assertFalse(
            self.superposition(**{"log.html": b"Benchmark stopped Benchmark running"})["ok"]
        )

    def test_superposition_click_derived_from_rectangle(self):
        script = m.render_check("superposition", superposition_run=True)
        self.assertIn("BoundingRectangle", script)
        self.assertIn("$rect.Left+$rect.Width*", script)
        self.assertIn("$rect.Top+$rect.Height*", script)
        self.assertIn("Remove-Item $source", script)

    def test_solidworks_default_launch_only(self):
        self.assertNotIn("OpenDoc6", m.render_check("solidworks"))
        self.assertIn("OpenDoc6", m.render_check("solidworks", solidworks_export=True))

    def test_resolve_default_launch_only(self):
        self.assertIn("$r.kind='LAUNCH_ONLY'", m.render_check("resolve"))

    def test_result_markdown_evidence_kinds(self):
        rows = [
            dict(step=kind, verdict="PASS", evidence=kind, detail="checked", **{"class": "app"})
            for kind in ("FILE", "UIA", "SCREEN")
        ]
        text = m.result_markdown(rows)
        for kind in ("FILE", "UIA", "SCREEN"):
            self.assertIn("| " + kind + " | PASS | " + kind + " |", text)

    def test_boot_age_boundary(self):
        self.assertFalse(m.boot_age_guard(3200, 100))
        self.assertTrue(m.boot_age_guard(3200, 101))

    def test_gpu_fix_failure(self):
        g = SimpleNamespace(
            ps=lambda s: SimpleNamespace(returncode=1, stdout="GPU-test", stderr="failure")
        )
        self.assertFalse(m.gpu_fix(g, "GPU-test")[0])

    def test_gpu_fix_wrong_uuid_even_rc_zero(self):
        g = SimpleNamespace(
            ps=lambda s: SimpleNamespace(returncode=0, stdout="GPU-other", stderr="")
        )
        self.assertFalse(m.gpu_fix(g, "GPU-test")[0])

    def test_interactive_task_cleanup_uses_registered_prefix(self):
        self.assertIn("$n='kf_app_", m.task_wrapper("app_abc"))
        # Deterministic task-start failure still must unregister its exact task.
        calls = []
        g = SimpleNamespace(
            interactive=lambda *a: SimpleNamespace(returncode=1), ps=lambda s: calls.append(s)
        )
        with (
            tempfile.TemporaryDirectory() as td,
            patch.object(m.uuid, "uuid4", return_value=SimpleNamespace(hex="a" * 32)),
        ):
            self.assertFalse(m.run_check("resolve", {}, Path(td), g)["ok"])
        self.assertIn("Unregister-ScheduledTask -TaskName kf_app_" + ("a" * 32), calls[-1])

    def test_timeout_is_harness_and_cleans_task(self):
        calls = []

        def ps(script):
            calls.append(script)
            return SimpleNamespace(returncode=0, stdout="", stderr="")

        g = SimpleNamespace(interactive=lambda *a: SimpleNamespace(returncode=0), ps=ps)
        times = iter([0, 0, 901])
        with tempfile.TemporaryDirectory() as td:
            row = m.run_check(
                "solidworks", {}, Path(td), g, clock=lambda: next(times), sleep=lambda _: None
            )
        self.assertFalse(row["ok"])
        self.assertEqual(row["class"], "harness")
        self.assertIn("Unregister-ScheduledTask -TaskName kf_app_", calls[-1])

    def test_main_rejects_old_swtpm_before_unpack(self):
        with (
            tempfile.TemporaryDirectory() as td,
            patch.object(
                m,
                "run",
                return_value=SimpleNamespace(
                    returncode=0, stdout="TPM emulator version 0.6.3", stderr=""
                ),
            ),
            patch.object(m, "load_image") as load,
            patch("sys.stdout", new=io.StringIO()),
        ):
            rc = m.main(
                ["--dir", td, "--work", td + "/run", "--out", td + "/out", "--qemu", "unused"]
            )
            self.assertEqual(rc, 1)
            load.assert_not_called()
            rows = json.loads((Path(td) / "out/result.json").read_text())
            self.assertEqual(rows[-1]["class"], "harness")
            self.assertIn("swtpm >= 0.10", rows[-1]["detail"])

    def test_multiline_guest_ps_uses_file_not_stdin(self):
        calls = []
        g = m.Guest({"guest": {"user": "kfimage"}}, "unused", 1)

        def ssh(args, **kw):
            calls.append(args)
            return SimpleNamespace(returncode=0, stdout="", stderr="")

        g.ssh = ssh
        g.scp = lambda *a: SimpleNamespace(returncode=0)
        g.ps("first line\nsecond line")
        self.assertIn("-File", calls[-1])
        self.assertNotIn("-Command", calls[-1])

    def test_harness_row_not_app_failure(self):
        self.assertEqual(
            m.app_classification({"class": "harness", "ok": False}), ("HARNESS", "harness")
        )

    def test_harness_error_preserved_in_result(self):
        self.assertEqual(
            m.app_classification({"class": "harness", "error": "transport lost"}),
            ("HARNESS", "harness"),
        )

    def test_launch_is_not_pass_in_pipeline(self):
        self.assertEqual(
            m.app_classification({"ok": False, "status": "LAUNCH_ONLY"}),
            ("LAUNCH_ONLY", "unqualified"),
        )

    def test_old_wlms_event_ignored_by_query(self):
        calls = []
        g = SimpleNamespace(
            ps=lambda text: (calls.append(text) or SimpleNamespace(returncode=0, stdout="[]"))
        )
        self.assertEqual(m.shutdown_events(g), [])
        self.assertIn("StartTime=$boot", calls[0])

    def pipeline(self, post_events=None, offline=True, overage=False):
        """Deterministic complete pipeline: no VM, keys, SSH, GPU or sleeps."""
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            work = root / "work"
            out = root / "out"
            flags = root / "flags.env"
            flags.write_text("KF3_TEST=1\n")
            state = {
                "guest": {"user": "kfimage", "ssh_private_key": "fixture-not-a-key"},
                "ids": {"gpu_uuid": "GPU-test", "net": {"ssh_port": 1}},
                "clock": {},
            }
            processes = []

            class Process:
                def __init__(self, cmd, **kw):
                    self.rc = None
                    processes.append(self)
                    if cmd[0] == "swtpm":
                        (work / "swtpm.sock").touch()

                def poll(self):
                    return self.rc

                def terminate(self):
                    self.rc = 0

                def kill(self):
                    self.rc = 0

                def wait(self, timeout):
                    self.rc = 0
                    return self.rc

            def runner(cmd, **kw):
                stdout = "TPM emulator version 0.10.1" if cmd[-1] == "--version" else ""
                if cmd[:2] == ["qemu-img", "info"]:
                    stdout = json.dumps({"full-backing-filename": str(work / "image.base.qcow2")})
                return SimpleNamespace(returncode=0, stdout=stdout, stderr="")

            def ps(script, **kw):
                if script == "Stop-Computer -Force":
                    processes[-1].rc = 0
                return SimpleNamespace(returncode=0, stdout="GPU-test", stderr="")

            g = SimpleNamespace(ps=ps, ssh=lambda *a, **k: SimpleNamespace(returncode=0))
            pi = SimpleNamespace(
                PropError=ValueError,
                restore_files=lambda *a: None,
                boot_clock=lambda *a: SimpleNamespace(isoformat=lambda: "fixture-clock"),
                cpu_preflight=lambda *a: [],
                run_guest_probe=lambda *a: {},
                compare=lambda *a: [("ids", "MATCH", "")],
                build_argv=lambda *a, **k: ["qemu", str(work / "run/clone.qcow2")]
                + (["restrict=on"] if offline else []),
            )
            with (
                patch.object(m, "PI", pi),
                patch.object(m, "load_image", return_value=state),
                patch.object(m, "run", side_effect=runner),
                patch.object(m.subprocess, "Popen", Process),
                patch.object(m, "Guest", return_value=g),
                patch.object(m.time, "sleep"),
                patch.object(
                    m.time,
                    "monotonic",
                    side_effect=[0, 1, 3400] if overage else None,
                    return_value=0,
                ),
                patch.object(m, "shutdown_events", side_effect=[[], post_events or []]),
                patch.object(
                    m, "run_check", return_value={"ok": True, "evidence": "FILE", "signal": "work"}
                ) as app,
                patch("sys.stdout", new=io.StringIO()),
            ):
                rc = m.main(
                    [
                        "--dir",
                        str(root / "img"),
                        "--work",
                        str(work),
                        "--out",
                        str(out),
                        "--qemu",
                        "qemu",
                        "--flags",
                        str(flags),
                        "--display",
                        "off",
                        "--apps",
                        "resolve",
                    ]
                )
            rows = json.loads((out / "result.json").read_text())
            self.assertTrue(all(pr.poll() is not None for pr in processes))
            self.assertFalse((work / "id_kf").exists())
            return rc, rows, app.call_count

    def test_pipeline_wlms_postrun_never_app_fail(self):
        rc, rows, _ = self.pipeline([{"event_id": 1074, "process": "wlms.exe"}])
        self.assertEqual(rc, 1)
        app = next(r for r in rows if r["step"] == "app:resolve")
        self.assertEqual((app["verdict"], app["class"]), ("HARNESS", "harness"))

    def test_pipeline_wlms_path_after_work_is_harness(self):
        _, rows, _ = self.pipeline([{"event_id": 1074, "process": r"C:\Windows\System32\WLMS.EXE"}])
        app = next(r for r in rows if r["step"] == "app:resolve")
        self.assertNotEqual(app["verdict"], "PASS")
        self.assertEqual(app["class"], "harness")

    def test_pipeline_rejects_online_argv_and_cleans_helpers(self):
        rc, rows, apps = self.pipeline(offline=False)
        self.assertEqual(rc, 1)
        self.assertEqual(apps, 0)
        self.assertIn("Offline overlay invariant violated", rows[-1]["detail"])

    def test_pipeline_boot_age_skips_work(self):
        rc, rows, apps = self.pipeline(overage=True)
        self.assertEqual(rc, 1)
        self.assertEqual(apps, 0)
        self.assertTrue(any(r["verdict"] == "NOT_RUN" and r["class"] == "harness" for r in rows))

    def test_pipeline_work_pass_has_evidence(self):
        rc, rows, apps = self.pipeline()
        self.assertEqual(rc, 0)
        self.assertEqual(apps, 1)
        app = next(r for r in rows if r["step"] == "app:resolve")
        self.assertEqual((app["verdict"], app["evidence"]), ("PASS", "FILE"))


def mutants():
    """In-memory, authored-source mutations; two independent test methods per rule."""
    global m
    original = m
    source = SRC.read_text()
    cases = [
        ("harness-class", 'return "HARNESS", "harness"', 'return "FAIL", "app"'),
        (
            "wlms-postrun",
            'license_shutdown = classify_shutdown(events) == "harness"',
            "license_shutdown = False",
        ),
        ("run-binding", 'r.get("run_id") != run_id', "False"),
        ("app-binding", 'r.get("app") != name', "False"),
        ("freshness", "not start < modified <= end", "False"),
        ("length", 'len(raw) != r.get("bytes")', "False"),
        ("api-success", 'r.get("api_ok") is not True', "False"),
        ("step-structure", 'raise ValueError("Invalid STEP exchange structure")', "pass"),
        ("progress-transition", 'record.get("progress") == running', "True"),
        ("prior-state", 'record.get("before") not in (None, "", running, finished)', "True"),
        ("swtpm-version", "version < (0, 10, 0)", "False"),
        ("gpu-instances", "Get-ChildItem $base", "Get-Item $base"),
        ("gpu-seed", "bool(actual.strip()) and actual.strip() == expected", "True"),
        ("wlms-event", 'e.get("event_id") == 1074', "True"),
        ("wlms-process", 're.search(r"(?i)\\bwlms\\.exe\\b", str(e.get("process", "")))', "True"),
        ("boot-age", "age + budget > 55 * 60", "False"),
        ("superposition-fresh", 'not start < r.get("modified", 0) <= end', "False"),
        (
            "task-cleanup",
            "Unregister-ScheduledTask -TaskName kf_{task}",
            "Unregister-ScheduledTask -TaskName {task}",
        ),
        (
            "superposition-screen",
            'raise ValueError("Result screen score/FPS not readable; no verdict")',
            "return",
        ),
    ]
    failed = False
    for name, old, new in cases:
        expected_count = 2 if name == "length" else 1
        if source.count(old) != expected_count:
            raise AssertionError((name, source.count(old)))
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "mutant_propimage_check.py"
            path.write_text(
                source.replace(old, new).replace(
                    "HERE = Path(__file__).resolve().parent", f"HERE = Path({str(SRC.parent)!r})"
                )
            )
            mutation_spec = importlib.util.spec_from_file_location("mutant_propimage_check", path)
            m = importlib.util.module_from_spec(mutation_spec)
            mutation_spec.loader.exec_module(m)
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(T)
        result = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
        killers = sorted({test.id().split(".")[-1] for test, _ in result.failures + result.errors})
        print(name, len(killers), ",".join(killers))
        failed |= len(killers) < 2
    m = original
    return int(failed)


if __name__ == "__main__":
    if sys.argv[1:] == ["--mutants"]:
        sys.exit(mutants())
    unittest.main()
