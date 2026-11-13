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

    def pipeline(
        self,
        post_events=None,
        offline=True,
        overage=False,
        power_ok=True,
        resume=False,
        clock_query_ok=True,
        resume_last="2026-11-12T00:10:00Z",
    ):
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
                "clock": {"last_time_at_stop_utc": "2026-11-12T00:00:00Z", "gap_s": 60},
            }
            processes = []
            used_bases = []
            if resume:
                (work / "run").mkdir(parents=True)
                (work / "run/clone.qcow2").touch()
                (work / "image.base.qcow2").symlink_to(root / "img/image.qcow2")
                (work / "run/clone.qcow2.clock").write_text(
                    json.dumps(
                        {
                            "base_utc": "2026-11-12T00:01:00Z",
                            "last_guest_utc": resume_last,
                            "boots": 1,
                        }
                    )
                )
                for old in ("swtpm.sock", "run/broker.sock", "run/broker.fifo"):
                    (work / old).touch()

            self_outer = self

            class Process:
                def __init__(self, cmd, **kw):
                    self.rc = None
                    processes.append(self)
                    if cmd[0] == "swtpm":
                        if resume:
                            self_outer.assertFalse((work / "run/broker.fifo").exists())
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
                stamp = (
                    (
                        m.PI.fmt(m.PI.utc(resume_last) + m.timedelta(seconds=120))
                        if resume
                        else "2026-11-12T00:02:00Z"
                    )
                    if "Get-Date" in script
                    else "GPU-test"
                )
                return SimpleNamespace(
                    returncode=0 if clock_query_ok or "Get-Date" not in script else 1,
                    stdout=stamp,
                    stderr="",
                )

            g = SimpleNamespace(ps=ps, ssh=lambda *a, **k: SimpleNamespace(returncode=0))
            pi = SimpleNamespace(
                PropError=ValueError,
                restore_files=lambda *a: None,
                boot_clock=m.PI.boot_clock,
                utc=m.PI.utc,
                fmt=m.PI.fmt,
                cpu_preflight=lambda *a: [],
                run_guest_probe=lambda *a: {},
                compare=lambda s, f: (
                    used_bases.append(m.PI.boot_clock(s, "ci")) or [("ids", "MATCH", "")]
                ),
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
                patch.object(
                    m,
                    "power_fix",
                    return_value={
                        "ok": power_ok,
                        "values": {"sleep": {"ac": 0, "dc": 0}},
                        "errors": [] if power_ok else ["readback mismatch"],
                    },
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
                    + (["--resume"] if resume else [])
                )
            rows = json.loads((out / "result.json").read_text())
            if used_bases:
                self.assertEqual(
                    m.PI.fmt(used_bases[0]),
                    (
                        m.PI.fmt(m.PI.utc(resume_last) + m.timedelta(seconds=60))
                        if resume
                        else "2026-11-12T00:01:00Z"
                    ),
                )
            if clock_query_ok and offline:
                data = json.loads((work / "run/clone.qcow2.clock").read_text())
                self.assertEqual(data["boots"], 2 if resume else 1)
                self.assertEqual(
                    data["last_guest_utc"],
                    (
                        m.PI.fmt(m.PI.utc(resume_last) + m.timedelta(seconds=120))
                        if resume
                        else "2026-11-12T00:02:00Z"
                    ),
                )
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

    def power_fixture(self):
        return {
            "queries": {name: "AC 0x00000000 DC 0x00000000" for name in m.POWER_SETTINGS},
            "registry": {name: entry[2] for name, entry in m.POWER_REGISTRY.items()},
            "errors": [],
            "restart_required": False,
            "hiberfile_exists": False,
            "sid": "S-1-5-test",
            "screen": {"active": False, "secure": False, "timeout": 0, "sid": "S-1-5-test"},
        }

    def test_power_valid_readback(self):
        self.assertTrue(m.power_verdict(self.power_fixture())["ok"])

    def test_power_ac_timeout_rejected(self):
        r = self.power_fixture()
        r["queries"]["sleep"] = "AC 0x00000001 DC 0x00000000"
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_dc_timeout_rejected(self):
        r = self.power_fixture()
        r["queries"]["disk"] = "AC 0x00000000 DC 0x00000005"
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_missing_query_rejected(self):
        r = self.power_fixture()
        del r["queries"]["monitor"]
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_missing_registry_rejected(self):
        r = self.power_fixture()
        del r["registry"]["fast_startup"]
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_hibernate_enabled_rejected(self):
        r = self.power_fixture()
        r["registry"]["hibernate_enabled"] = 1
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_fast_startup_rejected(self):
        r = self.power_fixture()
        r["registry"]["fast_startup"] = 1
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_screen_still_active_rejected(self):
        r = self.power_fixture()
        r["screen"]["active"] = True
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_wrong_session_rejected(self):
        r = self.power_fixture()
        r["screen"]["sid"] = "S-1-5-other"
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_native_failure_rejected(self):
        r = self.power_fixture()
        r["errors"] = ["powercfg exit 1"]
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_restart_required_rejected(self):
        r = self.power_fixture()
        r["restart_required"] = True
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_hiberfile_rejected(self):
        r = self.power_fixture()
        r["hiberfile_exists"] = True
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_render_writes_and_reads(self):
        script = m.render_power_fix()
        self.assertIn("/hibernate", script)
        self.assertIn("/setacvalueindex", script)
        self.assertIn("/setdcvalueindex", script)
        self.assertIn("/q", script)
        self.assertIn("Get-ItemPropertyValue", script)
        self.assertIn("$LASTEXITCODE", script)
        self.assertIn("HiberbootEnabled", script)

    def test_power_render_never_sleep(self):
        script = m.render_power_fix()
        for _, setting in m.POWER_SETTINGS.values():
            self.assertIn(setting, script)
        self.assertNotIn("Restart-Computer", script)

    def test_power_ignores_claimed_ok(self):
        self.assertFalse(m.power_verdict({"ok": True})["ok"])

    def test_power_argument_vector_keeps_all_arguments(self):
        # Real guest: ValueFromRemainingArguments + `Power @('/hibernate','off')` bound ONE joined string, so powercfg
        # got the single argument "/hibernate off" and answered "Invalid Parameters". A plain string[] keeps elements.
        self.assertNotIn("ValueFromRemainingArguments", m.render_power_fix())
        self.assertIn("param([string[]]$argsList)", m.render_power_fix())
        self.assertIn("powercfg @argsList", m.render_power_fix())

    def test_power_repeat_preserves_pending_restart(self):
        script = m.render_power_fix()
        self.assertIn("$priorBoot -ceq $boot", script)
        self.assertIn("InactivityRestartBoot", script)

    def test_power_native_failure_keeps_actual_values(self):
        r = self.power_fixture()
        r["errors"] = ["command failed"]
        result = m.power_verdict(r)
        self.assertFalse(result["ok"])
        self.assertIn("sleep", result["values"])

    def test_power_screensaver_secure_rejected(self):
        r = self.power_fixture()
        r["screen"]["secure"] = True
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_screensaver_timeout_rejected(self):
        r = self.power_fixture()
        r["screen"]["timeout"] = 300
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_identity_missing_rejected(self):
        r = self.power_fixture()
        del r["sid"]
        self.assertFalse(m.power_verdict(r)["ok"])

    def power_guest(self, data):
        screen = data.pop("screen")
        calls = []

        def ps(script):
            calls.append(script)
            stdout = (
                json.dumps(screen)
                if "Get-Content" in script
                else (json.dumps(data) if "ConvertTo-Json" in script else "")
            )
            return SimpleNamespace(returncode=0, stdout=stdout, stderr="")

        return (
            SimpleNamespace(ps=ps, interactive=lambda *args: SimpleNamespace(returncode=0)),
            calls,
        )

    def test_power_fix_reads_same_user_runtime(self):
        guest, calls = self.power_guest(self.power_fixture())
        result = m.power_fix(guest, sleep=lambda _: None, clock=lambda: 0)
        self.assertTrue(result["ok"])
        self.assertIn("Unregister-ScheduledTask -TaskName kf_power_", calls[-1])

    def test_power_fix_rejects_bad_registry(self):
        r = self.power_fixture()
        r["registry"]["fast_startup"] = 1
        guest, _ = self.power_guest(r)
        self.assertFalse(m.power_fix(guest, sleep=lambda _: None, clock=lambda: 0)["ok"])

    def test_power_fix_native_exit_failure(self):
        guest = SimpleNamespace(
            ps=lambda *a: SimpleNamespace(returncode=1, stdout="{}", stderr="failed")
        )
        self.assertFalse(m.power_fix(guest)["ok"])

    def test_power_pipeline_not_ok_blocks_apps(self):
        rc, rows, apps = self.pipeline(power_ok=False)
        self.assertEqual(rc, 1)
        self.assertEqual(apps, 0)
        power = next(r for r in rows if r["step"] == "power")
        self.assertEqual(power["verdict"], "NOT OK")
        self.assertIn('"dc":0', power["detail"])

    def test_power_pipeline_ok_with_values(self):
        rc, rows, _ = self.pipeline()
        self.assertEqual(rc, 0)
        power = next(r for r in rows if r["step"] == "power")
        self.assertEqual(power["verdict"], "OK")
        self.assertIn('"ac":0', power["detail"])

    def test_power_not_ok_blocks_gpu(self):
        _, rows, _ = self.pipeline(power_ok=False)
        self.assertFalse(any(r["step"] == "gpu" for r in rows))

    def test_power_ambiguous_query_is_not_readback(self):
        r = self.power_fixture()
        r["queries"]["sleep"] = "minimum 0x00000000 maximum 0x00000000"
        self.assertFalse(m.power_verdict(r)["ok"])

    def test_power_screen_error_rejected_even_with_values(self):
        r = self.power_fixture()
        r["screen"]["error"] = "SPI failed"
        self.assertFalse(m.power_verdict(r)["ok"])

    def clock_sidecar(self, root):
        return Path(root) / "overlay.qcow2"

    def test_optional_registry_reads_cannot_abort_the_repair(self):
        # Real guest: Get-ItemPropertyValue on a missing optional value throws a TERMINATING error that
        # -ErrorAction SilentlyContinue does not stop, so under $ErrorActionPreference='Stop' the repair aborted.
        script = m.render_power_fix()
        self.assertNotRegex(script, r"Get-ItemPropertyValue[^\n]*SilentlyContinue")
        self.assertIn("function Opt(", script)
        self.assertIn("Opt $marker 'InactivityRestartBoot'", script)
        self.assertIn("'InactivityTimeoutSecs'", script)

    def test_every_power_setting_resolves_without_a_missing_alias(self):
        # Real guest: powercfg /aliases lists SUB_* and most settings but NOT CONSOLELOCK, so Resolve-Guid threw
        # "Missing powercfg alias CONSOLELOCK". Settings that the alias list may lack are carried as GUIDs.
        guid = r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}"
        self.assertRegex(m.POWER_SETTINGS["wake_signin"][1], guid)
        self.assertNotIn("CONSOLELOCK", repr(m.POWER_SETTINGS))

    def test_screensaver_set_tolerates_policy_block_but_readback_decides(self):
        # Real guest: after the repair writes the screensaver policy, SystemParametersInfo(SET) fails with
        # 1260 (access disabled by policy) in the interactive task. Only that error is tolerated.
        script = m.render_screen_power("power_" + "a" * 32)
        self.assertIn("-ne 1260", script)
        self.assertIn("[KfPower]::Get(0x0010", script)
        self.assertIn("Cannot disable interactive screensaver", script)

    def test_gpu_fix_lists_class_keys_by_name(self):
        # Real guest: Get-ChildItem on the display class key opens the protected Properties subkey and throws
        # "Requested registry access is not allowed"; listing names only avoids it.
        script = m.render_gpu_fix("GPU-abc")
        self.assertIn("Get-ChildItem $base -Name", script)
        self.assertNotIn("$_.PSPath", script)

    def test_gop_efi_is_staged_from_dir_or_flag_and_missing_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "dir/firmware").mkdir(parents=True)
            (d / "dir/firmware/kf-gop.signed.efi").write_bytes(b"efi")
            out = m.stage_gop(d / "work", d / "dir")
            self.assertEqual(out.read_bytes(), b"efi")
            (d / "x.efi").write_bytes(b"flag")
            self.assertEqual(m.stage_gop(d / "w2", d / "dir", d / "x.efi").read_bytes(), b"flag")
            with self.assertRaises(ValueError):
                m.stage_gop(d / "w3", d / "none")

    def test_clock_fresh_uses_frozen_base(self):
        with tempfile.TemporaryDirectory() as td:
            frozen = m.PI.utc("2026-11-12T00:00:00Z")
            self.assertEqual(m.overlay_clock(self.clock_sidecar(td), frozen), frozen)

    def test_clock_sidecar_write_read(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            frozen = m.PI.utc("2026-11-12T00:00:00Z")
            m.overlay_clock(disk, frozen)
            m.overlay_clock(disk, frozen, guest_utc="2026-11-12T00:10:00Z")
            data = json.loads(Path(str(disk) + ".clock").read_text())
            self.assertEqual(data["last_guest_utc"], "2026-11-12T00:10:00Z")
            self.assertEqual(data["boots"], 1)
            self.assertEqual(m.overlay_clock(disk, frozen), m.PI.utc("2026-11-12T00:11:00Z"))

    def test_clock_later_frozen_reference_wins(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            m.overlay_clock(disk, m.PI.utc("2026-11-12T00:00:00Z"))
            m.overlay_clock(
                disk, m.PI.utc("2026-11-12T00:00:00Z"), guest_utc="2026-11-12T00:10:00Z"
            )
            later = m.PI.utc("2026-11-12T01:00:00Z")
            self.assertEqual(m.overlay_clock(disk, later), later)

    def test_clock_last_guest_never_decreases(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            frozen = m.PI.utc("2026-11-12T00:00:00Z")
            m.overlay_clock(disk, frozen)
            m.overlay_clock(disk, frozen, guest_utc="2026-11-12T00:10:00Z")
            m.overlay_clock(disk, frozen, guest_utc="2026-11-12T00:05:00Z")
            self.assertEqual(m.overlay_clock(disk, frozen), m.PI.utc("2026-11-12T00:11:00Z"))

    def test_clock_known_base_never_decreases(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            later = m.PI.utc("2026-11-12T01:00:00Z")
            m.overlay_clock(disk, later)
            self.assertEqual(m.overlay_clock(disk, m.PI.utc("2026-11-12T00:00:00Z")), later)

    def test_clock_corrupt_sidecar_warns_and_falls_back(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            frozen = m.PI.utc("2026-11-12T00:00:00Z")
            Path(str(disk) + ".clock").write_text("{broken")
            with self.assertWarnsRegex(RuntimeWarning, "frozen"):
                self.assertEqual(m.overlay_clock(disk, frozen, resume=True), frozen)

    def test_clock_missing_resume_sidecar_warns(self):
        with tempfile.TemporaryDirectory() as td:
            frozen = m.PI.utc("2026-11-12T00:00:00Z")
            with self.assertWarnsRegex(RuntimeWarning, "frozen"):
                self.assertEqual(
                    m.overlay_clock(self.clock_sidecar(td), frozen, resume=True), frozen
                )

    def test_clock_bad_schema_warns(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            frozen = m.PI.utc("2026-11-12T00:00:00Z")
            Path(str(disk) + ".clock").write_text(
                json.dumps({"base_utc": "bad", "last_guest_utc": None, "boots": -1})
            )
            with self.assertWarns(RuntimeWarning):
                self.assertEqual(m.overlay_clock(disk, frozen), frozen)

    def test_clock_resume_window(self):
        state = m.PI.init_state("clock-test")
        state["clock"] = {"last_time_at_stop_utc": "2026-11-12T02:00:00Z", "gap_s": 60}
        for stamp, want in [
            ("2026-11-12T02:01:00Z", "MATCH"),
            ("2026-11-12T02:31:00Z", "MATCH"),
            ("2026-11-12T02:31:01Z", "MISMATCH"),
            ("2026-11-12T00:01:00Z", "MISMATCH"),
        ]:
            row = next(r for r in m.PI.compare(state, {"date_utc": stamp}) if r[0] == "clock")
            self.assertEqual(row[1], want)

    def test_clock_pipeline_resume_uses_derived_window(self):
        rc, rows, _ = self.pipeline(resume=True)
        self.assertEqual(rc, 0)
        row = next(r for r in rows if r["step"] == "clock")
        self.assertIn("2026-11-12T00:11:00Z", row["detail"])

    def test_clock_pipeline_shutdown_capture_failure_warns(self):
        rc, rows, _ = self.pipeline(clock_query_ok=False)
        self.assertEqual(rc, 1)
        self.assertTrue(any(r["step"] == "clock-save" and r["verdict"] == "WARN" for r in rows))

    def test_clock_invalid_boot_counter_warns(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            frozen = m.PI.utc("2026-11-12T00:00:00Z")
            Path(str(disk) + ".clock").write_text(
                json.dumps(
                    {"base_utc": "2026-11-12T00:00:00Z", "last_guest_utc": None, "boots": True}
                )
            )
            with self.assertWarns(RuntimeWarning):
                self.assertEqual(m.overlay_clock(disk, frozen), frozen)

    def test_clock_inprocess_reboot_keeps_vm_rtc(self):
        state = m.PI.init_state("clock-test")
        state["clock"] = {"last_time_at_stop_utc": "2026-11-12T02:00:00Z", "gap_s": 60}
        argv = m.PI.build_argv(state, "ci", "overlay.qcow2", "/tmp/clock-fixture", display="off")
        self.assertIn("base=2026-11-12T02:01:00,clock=vm", argv[argv.index("-rtc") + 1])

    def test_clock_capture_not_before_known_base(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            base = m.PI.utc("2026-11-12T01:00:00Z")
            m.overlay_clock(disk, base)
            m.overlay_clock(disk, base, guest_utc="2026-11-12T00:10:00Z")
            self.assertEqual(
                json.loads(Path(str(disk) + ".clock").read_text())["last_guest_utc"],
                "2026-11-12T01:00:00Z",
            )

    def test_clock_capture_preserves_previous_rtc(self):
        with tempfile.TemporaryDirectory() as td:
            disk = self.clock_sidecar(td)
            m.overlay_clock(disk, m.PI.utc("2026-11-12T01:00:00Z"))
            m.overlay_clock(
                disk, m.PI.utc("2026-11-12T00:00:00Z"), guest_utc="2026-11-12T00:10:00Z"
            )
            self.assertEqual(
                json.loads(Path(str(disk) + ".clock").read_text())["base_utc"],
                "2026-11-12T01:00:00Z",
            )

    def test_clock_pipeline_late_resume_uses_new_window(self):
        rc, rows, _ = self.pipeline(resume=True, resume_last="2026-11-12T02:00:00Z")
        self.assertEqual(rc, 0)
        row = next(r for r in rows if r["step"] == "clock")
        self.assertIn("2026-11-12T02:01:00Z", row["detail"])


def mutants():
    """In-memory, authored-source mutations; two independent test methods per rule."""
    global m
    original = m
    source = SRC.read_text()
    cases = [
        (
            "clock-advance",
            "base = max(base, last + timedelta(seconds=60) if last else frozen)",
            "base = base",
        ),
        (
            "clock-save",
            "last = max(PI.utc(guest_utc), base, last or base)",
            "last = PI.utc(guest_utc)",
        ),
        ("clock-high-water", "base = max(frozen, base)", "base = frozen"),
        (
            "clock-window",
            'rtc_base - timedelta(seconds=state["clock"]["gap_s"])',
            'frozen - timedelta(seconds=state["clock"]["gap_s"])',
        ),
        ("clock-boots", 'data["boots"] += 1', 'data["boots"] += 0'),
        ("clock-warning", "if resume or path.exists():", "if False:"),
        ("clock-capture", "overlay_clock(clone, frozen, guest_utc=utc.stdout.strip())", "pass"),
        ("power-step", "power = power_fix(g)", 'power = {"ok": True, "values": {}, "errors": []}'),
        ("power-indices", "if ac != 0 or dc != 0:", "if False:"),
        (
            "power-registry",
            "if type(actual) is not type(expected) or actual != expected:",
            "if False:",
        ),
        (
            "power-runtime",
            'errors.append("Interactive session screensaver readback failed or wrong user")',
            "pass",
        ),
        (
            "power-boot-state",
            'errors.append("Hibernation file present, or inactivity-policy restart required/unknown")',
            "pass",
        ),
        ("power-native", 'errors.extend(str(error) for error in data["errors"])', "pass"),
        ("power-integration", 'if not power["ok"]:', "if False:"),
        ("power-aggregate", 'r["verdict"] in ("PASS", "OK")', 'r["verdict"] == "PASS"'),
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
        expected_count = 2 if name in ("length", "task-cleanup") else 1
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
