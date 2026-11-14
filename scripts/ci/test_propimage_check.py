#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Hand-authored fixtures; no VM, network, third-party code or credentials."""

import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SRC = Path(
    os.environ.get("KF_PROPIMAGE_CHECK_UNDER_TEST")  # red-then-green: point at the pre-fix module
    or Path(__file__).resolve().parents[2] / "scripts/bench/windows/propimage_check.py"
)
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
            # Resolve alone types a project name (keyboard tier, UIA found the dialog); nothing types a secret.
            banned = r"(?i)password|Invoke-WebRequest|Invoke-RestMethod" + ("" if n == "resolve" else r"|sendkeys")
            self.assertNotRegex(m.render_check(n), banned)

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

    def test_3dmark_accessibility_flag_is_required_not_an_experiment(self):
        # Measured 2026-11-14: with the flag the CEF UI exposes 163 named UIA elements within 60 s.
        self.assertIn("--force-renderer-accessibility", m.render_check("3dmark"))

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

    @staticmethod
    def sp_log(
        running="17:53:02",
        stopped="17:53:12",
        gpu="NVIDIA GeForce RTX 4070 32.0.15.8088 x1",
        desc="NVIDIA GeForce RTX 4070",
        extra="",
    ):
        """Shape of the real Superposition 1.1 log.html measured on the PC 2026-11-14 (no score, no FPS)."""
        lines = [
            '<div class="e">17:52:43 Can\'t initialize OpenAL. Install driver for your sound card.</div>',
            f'<div class="m">17:52:44 GPU: {gpu}</div>',
            f'<div class="m">17:52:44 Direct3D11 desc: {desc}</div>',
            '<div class="m">17:52:46 Preset: 1 (Medium)</div>',
        ]
        if running:
            lines.append(f'<div class="m">{running} Benchmark running</div>')
        if stopped:
            lines.append(f'<div class="m">{stopped} Benchmark stopped</div>')
        if extra:
            lines.append(extra)
        return ("<html><body>" + "\n".join(lines) + "</body></html>").encode()

    def superposition(self, **changes):
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            log = self.sp_log()
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
                    if value is None:
                        (d / key).unlink()
                    else:
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

    def test_superposition_verdict_comes_from_the_file_not_the_ocr(self):
        # Measured 2026-11-14: tesseract returned noise ("eum aideit idils piles") for a real, clean
        # result screen of a completed run. The log is the verdict; unreadable OCR cannot veto it.
        garbage = "eum aideit\nidils piles\n@ SUPERPOSITION a mae\n9 BENCHMARK"
        row = self.superposition(**{"screen.txt": garbage})
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["evidence"], "FILE")
        self.assertIn("GeForce RTX 4070", row["signal"])
        self.assertIn("10 s", row["signal"])

    def test_superposition_without_ocr_still_file_verdict(self):
        row = self.superposition(**{"screen.txt": None})
        self.assertTrue(row["ok"], row)
        self.assertIn("score and FPS are not in the log file", row["signal"])

    def test_superposition_readable_screen_values_are_reported_not_required(self):
        row = self.superposition(**{"screen.txt": "Score 7996 FPS: Min 36.59, Avg 59.81, Max 61.18"})
        self.assertTrue(row["ok"], row)
        self.assertIn("screen read", row["signal"])
        self.assertIn("59.81", row["signal"])

    def test_superposition_bare_ocr_number_is_not_reported_as_the_score(self):
        # Measured 2026-11-14: OCR noise "2076)" next to a real score of about 8000 was reported as the score.
        row = self.superposition(**{"screen.txt": "2076)\nFPS: Min 33.99, Avg 59.81, Max 62.37"})
        self.assertTrue(row["ok"], row)
        self.assertNotIn("2076", row["signal"])

    def test_superposition_zero_fps_on_screen_is_a_failure(self):
        row = self.superposition(**{"screen.txt": "Score 12 FPS Min 0 Avg 0 Max 0"})
        self.assertFalse(row["ok"])

    def test_superposition_unordered_fps_on_screen_is_a_failure(self):
        row = self.superposition(**{"screen.txt": "Score 12 FPS Min 90 Avg 50 Max 70"})
        self.assertFalse(row["ok"])

    def test_superposition_unchanged_screen(self):
        self.assertFalse(self.superposition(**{"result.png": b"\x89PNG\r\n\x1a\nbefore"})["ok"])

    def test_superposition_no_completed_interval(self):
        self.assertFalse(self.superposition(**{"log.html": b"Benchmark running"})["ok"])

    def test_superposition_reversed_interval(self):
        self.assertFalse(
            self.superposition(**{"log.html": b"Benchmark stopped Benchmark running"})["ok"]
        )

    def software(self, name):
        log = self.sp_log(gpu=name + " 1.0 x1", desc=name)
        return self.superposition(**{"log.html": log})["ok"]

    def test_superposition_basic_render_driver_is_not_gpu_work(self):
        self.assertFalse(self.software("NVIDIA Microsoft Basic Render Driver"))

    def test_superposition_llvmpipe_is_not_gpu_work(self):
        self.assertFalse(self.software("NVIDIA llvmpipe (LLVM 15)"))

    def test_superposition_warp_is_not_gpu_work(self):
        self.assertFalse(self.software("NVIDIA WARP"))

    def test_superposition_gpu_must_be_nvidia(self):
        log = self.sp_log(gpu="AMD Radeon RX 7800 1.0 x1", desc="AMD Radeon RX 7800")
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_intel_gpu_is_not_the_nvidia_device(self):
        log = self.sp_log(gpu="Intel(R) UHD Graphics 770 31.0.101.4 x1", desc="Intel(R) UHD Graphics 770")
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_renderer_prefix_of_another_nvidia_gpu(self):
        log = self.sp_log(gpu="NVIDIA GeForce RTX 3060 Ti 32.0.15.8088 x1", desc="NVIDIA GeForce RTX 4070")
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_missing_renderer_line(self):
        log = self.sp_log().replace(b"Direct3D11 desc:", b"desc-less:")
        row = self.superposition(**{"log.html": log})
        self.assertFalse(row["ok"])
        self.assertIn("names no GPU", row["error"])

    def test_superposition_renderer_must_match_reported_gpu(self):
        log = self.sp_log(desc="NVIDIA GeForce RTX 3060")
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_missing_gpu_line(self):
        log = self.sp_log().replace(b"GPU:", b"gpu-less:")
        row = self.superposition(**{"log.html": log})
        self.assertFalse(row["ok"])
        self.assertIn("names no GPU", row["error"])

    def test_superposition_fatal_log_line(self):
        log = self.sp_log(extra='<div class="f">17:53:05 Fatal error: device lost</div>')
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_interval_longer_than_the_run_is_stale_text(self):
        log = self.sp_log(running="17:53:02", stopped="17:55:58")  # 176 s inside a 20 s run
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_interval_much_longer_than_the_run(self):
        log = self.sp_log(running="10:00:00", stopped="10:59:59")
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_two_fatal_lines(self):
        extra = '<div class="f">1 x</div><div class="f">2 y</div>'
        self.assertFalse(self.superposition(**{"log.html": self.sp_log(extra=extra)})["ok"])

    def test_superposition_stop_before_start_is_not_a_midnight_wrap_of_a_short_run(self):
        # 17:53:12 -> 17:53:02 would be 86390 s across midnight: far beyond any run window
        log = self.sp_log(running="17:53:12", stopped="17:53:02")
        self.assertFalse(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_interval_across_midnight(self):
        log = self.sp_log(running="23:59:55", stopped="00:00:05")
        self.assertTrue(self.superposition(**{"log.html": log})["ok"])

    def test_superposition_zero_length_interval(self):
        log = self.sp_log(running="17:53:02", stopped="17:53:02")
        row = self.superposition(**{"log.html": log})
        self.assertFalse(row["ok"])
        self.assertIn("positive length", row["error"])

    def test_superposition_zero_length_interval_late_evening(self):
        log = self.sp_log(running="23:59:59", stopped="23:59:59")
        self.assertIn("positive length", self.superposition(**{"log.html": log})["error"])

    def test_superposition_short_interval_across_midnight(self):
        log = self.sp_log(running="23:59:50", stopped="00:00:00")
        self.assertTrue(self.superposition(**{"log.html": log})["ok"])

    EDGE_RUN_ID = "a" * 32

    def edge(self, title=None, **changes):
        """Edge verdict from a result.json shaped like the guest script's output."""
        if title is None:
            title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 (0x00002786) Direct3D11 vs_5_0 ps_5_0, D3D11) frames=238"
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            record = self.fixture(
                app="edge", kind="UIA", launched=True, window_title=title + " and 1 more page - Microsoft Edge"
            )
            for key in ("artifact", "modified", "bytes", "api_ok"):
                record.pop(key)
            record.update(changes)
            (d / "result.json").write_text(json.dumps(record))
            return m.verdict("edge", self.EDGE_RUN_ID, d)

    def test_edge_webgl_title_read_through_uia_passes(self):
        # Measured 2026-11-14: the old probe said launched:false for a window that was visibly open,
        # because msedge.exe hands over to the browser process and the launcher PID exits at once.
        row = self.edge()
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["evidence"], "UIA")
        self.assertIn("238 frames", row["signal"])

    def test_edge_window_missing_is_not_a_pass(self):
        row = self.edge(launched=False, window_title="")
        self.assertFalse(row["ok"])
        self.assertIn("window not found", row["error"])

    def test_edge_window_without_the_page_title_is_not_a_pass(self):
        row = self.edge(title="New tab")
        self.assertFalse(row["ok"])
        self.assertIn("did not report a renderer", row["error"])

    def test_edge_error_title_is_not_a_pass(self):
        row = self.edge(title="KFGL " + self.EDGE_RUN_ID + " err TypeError: gl is null")
        self.assertFalse(row["ok"])

    def test_edge_title_of_another_run(self):
        title = "KFGL " + "b" * 32 + " ok renderer=ANGLE (NVIDIA, NVIDIA GeForce RTX 4070) frames=10"
        self.assertIn("another run", self.edge(title=title)["error"])

    def test_edge_title_nonce_is_bound_to_the_run(self):
        # a stale title from a previous page of the same profile must not pass
        title = "KFGL " + "0" * 32 + " ok renderer=ANGLE (NVIDIA, NVIDIA GeForce RTX 4070) frames=10"
        self.assertFalse(self.edge(title=title)["ok"])

    def test_edge_zero_frames(self):
        title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (NVIDIA, NVIDIA GeForce RTX 4070) frames=0"
        row = self.edge(title=title)
        self.assertFalse(row["ok"])
        self.assertIn("no frames", row["error"])

    def test_edge_zero_frames_with_leading_zeros(self):
        title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (NVIDIA, NVIDIA GeForce RTX 4070) frames=000"
        self.assertFalse(self.edge(title=title)["ok"])

    def test_edge_launched_must_be_the_boolean_true(self):
        self.assertFalse(self.edge(launched="yes")["ok"])

    def test_edge_swiftshader_is_software(self):
        title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver) frames=50"
        self.assertIn("software", self.edge(title=title)["error"])

    def test_edge_nvidia_named_software_rasterizer_is_software(self):
        title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (NVIDIA, NVIDIA llvmpipe, D3D11) frames=50"
        self.assertIn("software", self.edge(title=title)["error"])

    def test_edge_basic_render_driver_is_not_nvidia(self):
        title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (Microsoft, Microsoft Basic Render Driver Direct3D11) frames=50"
        self.assertFalse(self.edge(title=title)["ok"])

    def test_edge_intel_renderer_is_not_the_nvidia_device(self):
        title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (Intel, Intel(R) UHD Graphics 770, D3D11) frames=50"
        self.assertIn("not the NVIDIA device", self.edge(title=title)["error"])

    def test_edge_amd_renderer_is_not_the_nvidia_device(self):
        title = "KFGL " + self.EDGE_RUN_ID + " ok renderer=ANGLE (AMD, AMD Radeon RX 7800, D3D11) frames=50"
        self.assertIn("not the NVIDIA device", self.edge(title=title)["error"])

    def test_edge_script_finds_the_window_by_process_and_title_not_the_launcher_pid(self):
        script = m.render_check("edge", run_id=self.EDGE_RUN_ID)
        self.assertIn("Win32_Process", script)
        self.assertIn("--user-data-dir", script)
        self.assertIn("ProcessIdProperty", script)
        self.assertIn("-like 'KFGL *'", script)
        self.assertNotIn("HasExited", script)  # the launcher PID exits by design
        self.assertIn(self.EDGE_RUN_ID, script.replace("__NONCE__", self.EDGE_RUN_ID))
        self.assertNotIn("SetCursorPos", script)

    def test_edge_script_closes_only_its_own_profile(self):
        script = m.render_check("edge", run_id=self.EDGE_RUN_ID)
        self.assertIn("Stop-Process -Id $_.ProcessId", script)
        self.assertIn("*$profile*", script)

    def test_interactive_task_window_is_hidden_so_screenshots_show_the_app(self):
        # Measured 2026-11-14: the visible PowerShell console covered most of every app screenshot.
        wrapper = m.task_wrapper("x")
        self.assertIn("-WindowStyle Hidden", wrapper)

    def test_interactive_task_hidden_flag_is_before_the_script_file(self):
        wrapper = m.task_wrapper("x")
        self.assertLess(wrapper.index("-WindowStyle Hidden"), wrapper.index("-File"))

    def wukong(self, **changes):
        r = self.fixture(
            app="wukong", kind="WUKONG", shipping_seen=True, shipping_secs=97, exited=True,
            vram_peak_mib=2578, vram_total_mib=4096, crash=None, kernel_events=[], app_errors=[],
        )
        for key in ("artifact", "modified", "bytes", "api_ok"):
            r.pop(key)
        r.update(changes)
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            (d / "result.json").write_text(json.dumps(r))
            return m.verdict("wukong", "a" * 32, d)

    CRASH = {
        "type": "GPUCrash", "message": "GPU Crash dump Triggered", "exe": "b1-Win64-Shipping",
        "gpu": "NVIDIA GeForce RTX 4070", "modules": ["b1_Win64_Shipping", "sl_dlss_g", "sl_interposer"],
    }

    def test_wukong_measured_gpu_crash_is_a_failure_with_the_signature(self):
        # Measured 2026-11-14 on the PC: GPUCrash 97 s after start, LiveKernelEvent 141 at the same second.
        row = self.wukong(crash=self.CRASH, kernel_events=[{"code": "141"}])
        self.assertFalse(row["ok"])
        self.assertEqual(row["evidence"], "FILE")
        self.assertEqual(m.app_classification(row), ("FAIL", "app"))
        for part in ("GPUCrash", "GPU Crash dump Triggered", "sl_dlss_g", "141", "4096", "2578"):
            self.assertIn(part, row["error"])

    def test_wukong_crash_without_kernel_event_still_fails(self):
        self.assertEqual(m.app_classification(self.wukong(crash=self.CRASH))[0], "FAIL")

    def test_wukong_kernel_watchdog_without_crash_context_fails(self):
        row = self.wukong(exited=False, kernel_events=[{"code": "117"}])
        self.assertEqual(m.app_classification(row)[0], "FAIL")
        self.assertIn("117", row["error"])

    def test_wukong_unrelated_kernel_event_is_ignored(self):
        row = self.wukong(exited=False, kernel_events=[{"code": "1a8"}])
        self.assertEqual(m.app_classification(row)[0], "INCONCLUSIVE")

    def test_wukong_never_started_is_a_failure(self):
        row = self.wukong(shipping_seen=False)
        self.assertEqual(m.app_classification(row)[0], "FAIL")

    def test_wukong_application_error_event_after_exit_fails(self):
        row = self.wukong(app_errors=[{"text": "Faulting application b1-Win64-Shipping.exe"}])
        self.assertEqual(m.app_classification(row)[0], "FAIL")

    def test_wukong_seen_flag_must_be_boolean_true(self):
        self.assertEqual(m.app_classification(self.wukong(shipping_seen="yes"))[0], "FAIL")

    def test_wukong_two_application_error_events_fail(self):
        events = [{"text": "a"}, {"text": "b"}]
        row = self.wukong(app_errors=events)
        self.assertIn("Application Error", row["error"])

    def test_wukong_survival_alone_is_inconclusive_not_a_pass(self):
        row = self.wukong(exited=False)
        self.assertFalse(row["ok"])
        self.assertEqual(m.app_classification(row), ("INCONCLUSIVE", "unqualified"))

    def test_wukong_clean_exit_without_result_file_is_inconclusive(self):
        self.assertEqual(m.app_classification(self.wukong())[0], "INCONCLUSIVE")

    def test_wukong_script_launches_the_exe_with_the_appid_from_the_manifest(self):
        script = m.render_check("wukong", run_id="a" * 32)
        self.assertIn("b1_benchmark.exe", script)
        self.assertIn("appmanifest_*.acf", script)
        self.assertIn("$env:SteamAppId=$appid", script)
        self.assertNotIn(".url", script)
        self.assertNotIn("SetCursorPos", script)

    def test_wukong_script_captures_vram_and_event_context(self):
        script = m.render_check("wukong", run_id="a" * 32)
        for needle in ("Win32_VideoController", "nvidia-smi -q -d MEMORY", "dxdiag", "system_events.txt", "nvlddmkm"):
            self.assertIn(needle, script)

    def test_host_log_excerpt_filters_and_bounds(self):
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text("\n".join(["quiet line"] * 5 + [f"Xid {i} refused" for i in range(10)] + ["pramin ok"]))
            out = m.host_log_excerpt(log, limit=4)
            self.assertEqual(len(out), 4)
            self.assertTrue(all(("Xid" in x or "pramin" in x) for x in out))
            self.assertNotIn("quiet line", out)

    def test_host_log_excerpt_keeps_the_latest_lines(self):
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text("\n".join(f"error {i}" for i in range(50)))
            self.assertEqual(m.host_log_excerpt(log, limit=2), ["error 48", "error 49"])

    def test_host_log_excerpt_long_line_keeps_its_start(self):
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text("fault START " + "y" * 900)
            self.assertTrue(m.host_log_excerpt(log)[0].startswith("fault START"))
            self.assertEqual(len(m.host_log_excerpt(log)[0]), 400)

    def test_host_log_excerpt_skips_quiet_status_lines(self):
        quiet = "kf3: family=Ada phase=Running trapped=1 applied=2 refused=0 ram_refused=0 unshadowed_writes=0 read_exits=0"
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text(quiet + "\n" + quiet + "\nXid 79 device lost\n")
            self.assertEqual(m.host_log_excerpt(log), ["Xid 79 device lost"])

    def test_host_log_excerpt_skips_quiet_status_line_without_other_noise(self):
        quiet = "kf3: family=Blackwell phase=Running refused=0 ram_refused=0 unshadowed_writes=0 read_exits=0 mem[error_free=1]"
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text(quiet + "\n")
            self.assertEqual(m.host_log_excerpt(log), [])

    def test_host_log_excerpt_keeps_status_line_with_nonzero_refusals(self):
        bad = "kf3: family=Ada phase=Running trapped=1 applied=2 refused=3 ram_refused=0 unshadowed_writes=0"
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text(bad + "\n")
            self.assertEqual(m.host_log_excerpt(log), [bad])

    def test_host_log_excerpt_keeps_status_line_with_unshadowed_writes(self):
        bad = "kf3: family=Ada phase=Running refused=0 unshadowed_writes=7"
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text(bad + "\n")
            self.assertEqual(m.host_log_excerpt(log), [bad])

    def test_host_log_excerpt_missing_file_is_empty(self):
        self.assertEqual(m.host_log_excerpt("/nonexistent/qemu.log"), [])

    def test_host_log_excerpt_truncates_long_lines(self):
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "q.log"
            log.write_text("error " + "x" * 5000)
            self.assertLessEqual(len(m.host_log_excerpt(log)[0]), 400)

    def test_wukong_check_points_at_the_exe_not_the_url_shortcut(self):
        self.assertTrue(m.CHECKS["wukong"]["exe"].endswith("b1_benchmark.exe"))

    def test_wukong_script_collects_crash_context_and_kernel_events(self):
        script = m.render_check("wukong", run_id="a" * 32)
        for needle in ("CrashContext.runtime-xml", "LiveKernelEvent", "nvidia-smi", "Id=1000"):
            self.assertIn(needle, script)

    def test_run_replaces_undecodable_guest_bytes(self):
        # Measured 2026-11-14: a binary byte in guest output raised UnicodeDecodeError in the harness.
        p = m.run([sys.executable, "-c", "import sys;sys.stdout.buffer.write(b'ok\\x87\\x98end')"])
        self.assertEqual(p.returncode, 0)
        self.assertTrue(p.stdout.startswith("ok") and p.stdout.endswith("end"))

    def test_run_replaces_undecodable_bytes_on_stderr_too(self):
        p = m.run([sys.executable, "-c", "import sys;sys.stderr.buffer.write(b'e\\xff')"])
        self.assertTrue(p.stderr.startswith("e"))

    def test_run_keeps_bytes_mode_when_asked(self):
        p = m.run([sys.executable, "-c", "print(1)"], text=False)
        self.assertEqual(p.stdout.strip(), b"1")

    # ---- 3DMark result file (zip of Result.xml, Arielle.xml, SI.xml), structure measured 2026-11-14 ----
    RESULT_XML = (
        "<benchmark><results><result><name/><passIndex>-1</passIndex>"
        "<TimeSpyPerformanceGraphicsScore>7203</TimeSpyPerformanceGraphicsScore>"
        "<TimeSpyPerformance3DMarkScore>7237</TimeSpyPerformance3DMarkScore>"
        "<TimeSpyPerformanceCPUScore>7442</TimeSpyPerformanceCPUScore></result>"
        "<result><passIndex>0</passIndex>"
        "<TimeSpyPerformanceGraphicsTest1>47.66</TimeSpyPerformanceGraphicsTest1>"
        "<TimeSpyPerformanceGraphicsTest2>40.76</TimeSpyPerformanceGraphicsTest2></result>"
        "</results></benchmark>"
    )
    ARIELLE_XML = (
        "<benchmark><application_info><info><name>benchmark_start_time</name>"
        "<value>2026-11-12T17:55:00.000000000Z</value></info><info><name>benchmark_finish_time</name>"
        "<value>2026-11-12T18:02:10.000000000Z</value></info></application_info>"
        '<sets><set><workloads><workload><results><result pass_index="0"><status code="0"></status></result>'
        '</results></workload><workload><results><result pass_index="0"><status code="0"></status></result>'
        "</results></workload></workloads></set></sets></benchmark>"
    )
    SI_XML = (
        "<SystemInfo><Device><DeviceID>PCI\\VEN_10DE&amp;DEV_2786</DeviceID><Value><Name>Class</Name>"
        "<Value>Display</Value></Value><Value><Name>DeviceDesc</Name><Value>NVIDIA GeForce RTX 4070</Value>"
        "</Value><Software_Key><Value><Name>DriverVersion</Name><Value>32.0.15.8088</Value></Value></Software_Key>"
        "</Device></SystemInfo>"
    )
    # guest epoch for 2026-11-12T17:50:00Z .. 18:10:00Z
    TDM_START, TDM_END = 1794505800.0, 1794507000.0

    def refused(self, out, part):
        self.assertIsInstance(out, ValueError, out)
        self.assertIn(part, str(out))

    def tdm(self, result=None, arielle=None, si=None, extra=None, start=None, end=None, raw=None):
        import zipfile
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "r.3dmark-result"
            if raw is not None:
                path.write_bytes(raw)
            else:
                with zipfile.ZipFile(path, "w") as z:
                    z.writestr("Result.xml", self.RESULT_XML if result is None else result)
                    z.writestr("Arielle.xml", self.ARIELLE_XML if arielle is None else arielle)
                    z.writestr("SI.xml", self.SI_XML if si is None else si)
                    for name, data in (extra or {}).items():
                        z.writestr(name, data)
            try:
                return m.validate_3dmark_result(
                    path, self.TDM_START if start is None else start, self.TDM_END if end is None else end
                )
            except ValueError as exc:
                return exc

    def test_3dmark_result_file_passes_with_score_gpu_and_driver(self):
        out = self.tdm()
        self.assertNotIsInstance(out, Exception, out)
        for part in ("7237", "7203", "47.66", "NVIDIA GeForce RTX 4070", "32.0.15.8088"):
            self.assertIn(part, out)

    def test_3dmark_stale_result_from_the_image_is_refused(self):
        # the image ships a 2026-11-12T15:56 Time Spy result from preparation; it must never pass
        old = self.ARIELLE_XML.replace("17:55:00", "15:49:44").replace("18:02:10", "15:56:52")
        out = self.tdm(arielle=old)
        self.assertIsInstance(out, ValueError)
        self.refused(out, "not from this run")

    def test_3dmark_finish_after_the_run_end_is_refused(self):
        out = self.tdm(end=self.TDM_END - 3600)
        self.refused(out, "not from this run")

    def test_3dmark_start_before_the_run_start_is_refused(self):
        out = self.tdm(start=self.TDM_START + 600)
        self.refused(out, "not from this run")

    def test_3dmark_nonzero_workload_status_is_a_failure(self):
        bad = self.ARIELLE_XML.replace('<status code="0">', '<status code="9">', 1)
        self.refused(self.tdm(arielle=bad), "workload status")

    def test_3dmark_all_statuses_must_be_zero(self):
        head, _, tail = self.ARIELLE_XML.rpartition('<status code="0">')
        bad = head + '<status code="3">' + tail  # only the last workload failed
        self.refused(self.tdm(arielle=bad), "workload status")

    def test_3dmark_no_status_at_all_is_not_a_completed_run(self):
        none = self.ARIELLE_XML.replace('<status code="0"></status>', "")
        self.refused(self.tdm(arielle=none), "workload status")

    def test_3dmark_zero_score_is_refused(self):
        bad = self.RESULT_XML.replace(">7237<", ">0<")
        self.refused(self.tdm(result=bad), "score")

    def test_3dmark_missing_score_is_refused(self):
        bad = self.RESULT_XML.replace("TimeSpyPerformance3DMarkScore", "Other")
        self.refused(self.tdm(result=bad), "score")

    def test_3dmark_zero_graphics_fps_is_refused(self):
        bad = self.RESULT_XML.replace(">47.66<", ">0.00<")
        self.refused(self.tdm(result=bad), "FPS")

    def test_3dmark_no_fps_values_is_refused(self):
        bad = self.RESULT_XML.replace("GraphicsTest1", "X1").replace("GraphicsTest2", "X2")
        self.refused(self.tdm(result=bad), "FPS")

    def test_3dmark_missing_nvidia_display_device_is_refused(self):
        bad = self.SI_XML.replace("NVIDIA GeForce RTX 4070", "Microsoft Basic Render Driver")
        self.refused(self.tdm(si=bad), "NVIDIA")

    def test_3dmark_software_device_named_nvidia_is_refused(self):
        bad = self.SI_XML.replace("NVIDIA GeForce RTX 4070", "NVIDIA WARP")
        self.refused(self.tdm(si=bad), "software")

    def test_3dmark_intel_only_display_is_refused(self):
        bad = self.SI_XML.replace("NVIDIA GeForce RTX 4070", "Intel(R) UHD Graphics 770")
        self.refused(self.tdm(si=bad), "NVIDIA")

    def test_3dmark_non_display_class_nvidia_device_is_refused(self):
        bad = self.SI_XML.replace("<Value>Display</Value>", "<Value>Audio</Value>")
        self.refused(self.tdm(si=bad), "NVIDIA")

    def test_3dmark_llvmpipe_named_nvidia_is_software(self):
        bad = self.SI_XML.replace("NVIDIA GeForce RTX 4070", "NVIDIA llvmpipe")
        self.refused(self.tdm(si=bad), "software")

    def test_3dmark_not_a_zip_is_refused(self):
        self.refused(self.tdm(raw=b"not a zip"), "zip")

    def test_3dmark_missing_member_is_refused(self):
        import zipfile
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "r"
            with zipfile.ZipFile(path, "w") as z:
                z.writestr("Result.xml", self.RESULT_XML)
            with self.assertRaises(ValueError):
                m.validate_3dmark_result(path, self.TDM_START, self.TDM_END)

    def test_3dmark_xml_with_entities_is_refused_as_data_attack(self):
        evil = '<!DOCTYPE x [<!ENTITY a "boom">]>' + self.RESULT_XML
        self.refused(self.tdm(result=evil), "DOCTYPE")

    def test_3dmark_arielle_with_entities_is_refused(self):
        evil = '<!DOCTYPE x [<!ENTITY a "boom">]>' + self.ARIELLE_XML
        self.refused(self.tdm(arielle=evil), "DOCTYPE")

    def test_3dmark_unexpected_zip_member_path_is_refused(self):
        out = self.tdm(extra={"../evil.xml": "x"})
        self.refused(out, "member")

    def test_3dmark_oversized_member_is_refused(self):
        out = self.tdm(extra={"Monitoring.csv": b"x" * (64 * 1024 * 1024 + 1)})
        self.refused(out, "member")

    def test_3dmark_cli_refusal_text_is_recognised(self):
        self.assertTrue(m.cli_refused("18:14:02 Professional or Developer license key required for command line access"))
        self.assertFalse(m.cli_refused("18:14:02 benchmark finished"))

    def tdm_row(self, **changes):
        import zipfile
        with tempfile.TemporaryDirectory() as td:
            d = Path(td)
            with zipfile.ZipFile(d / "result.3dmark-result", "w") as z:
                z.writestr("Result.xml", self.RESULT_XML)
                z.writestr("Arielle.xml", self.ARIELLE_XML)
                z.writestr("SI.xml", self.SI_XML)
            size = (d / "result.3dmark-result").stat().st_size
            rec = self.fixture(
                app="3dmark", kind="TDM", started=self.TDM_START, ended=self.TDM_END,
                modified=self.TDM_START + 500, bytes=size, result_file="3DMark-TimeSpy-7237-x.3dmark-result",
                progress_seen=True, ui_texts="", cli_output="Professional or Developer license key required for command line access",
            )
            for key in ("artifact", "api_ok"):
                rec.pop(key)
            rec.update(changes)
            (d / "result.json").write_text(json.dumps(rec))
            return m.verdict("3dmark", "a" * 32, d)

    def test_3dmark_verdict_from_the_saved_file(self):
        row = self.tdm_row()
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["evidence"], "FILE")
        self.assertIn("7237", row["signal"])
        self.assertIn("CLI refused", row["signal"])

    def test_3dmark_no_file_after_progress_is_a_failure(self):
        row = self.tdm_row(result_file="")
        self.assertEqual(m.app_classification(row), ("FAIL", "app"))
        self.assertIn("saved no result file", row["error"])

    def test_3dmark_never_started_is_inconclusive_with_the_blocker_text(self):
        row = self.tdm_row(result_file="", progress_seen=False, ui_texts="Sign in to continue ; Offline")
        self.assertEqual(m.app_classification(row), ("INCONCLUSIVE", "unqualified"))
        self.assertIn("Sign in to continue", row["error"])

    def test_3dmark_stale_file_modified_before_the_run(self):
        self.assertIn("Stale", self.tdm_row(modified=self.TDM_START - 5)["error"])

    def test_3dmark_file_modified_after_the_run(self):
        self.assertIn("Stale", self.tdm_row(modified=self.TDM_END + 5)["error"])

    def test_3dmark_wrong_length_is_refused(self):
        self.assertIn("length", self.tdm_row(bytes=1)["error"])

    def test_3dmark_result_name_must_be_a_plain_result_file(self):
        self.assertIn("name", self.tdm_row(result_file="..\\evil.3dmark-result")["error"])
        self.assertIn("name", self.tdm_row(result_file="x.exe")["error"])

    def test_3dmark_never_started_with_empty_ui_text_is_inconclusive(self):
        row = self.tdm_row(result_file="", progress_seen=False, ui_texts="")
        self.assertEqual(m.app_classification(row)[0], "INCONCLUSIVE")

    def test_3dmark_result_name_exe_extension_refused(self):
        self.assertIn("name", self.tdm_row(result_file="x.exe")["error"])

    def test_3dmark_result_name_with_path_separator_refused(self):
        self.assertIn("name", self.tdm_row(result_file="a/b.3dmark-result")["error"])

    def test_3dmark_script_uses_uia_geometry_not_coordinates(self):
        script = m.render_check("3dmark", run_id="a" * 32)
        for needle in ("InvokePattern", "BoundingRectangle", "3DMarkCmd.exe", "*.3dmark-result"):
            self.assertIn(needle, script)
        self.assertNotIn("SetCursorPos", script)
        self.assertNotIn("mouse_event", script)
        self.assertNotIn("SendKeys", script)

    def test_3dmark_script_never_goes_online(self):
        script = m.render_check("3dmark", run_id="a" * 32)
        self.assertIn("--online=off", script)
        self.assertNotIn("INSTALL", script)

    def test_edge_page_is_authored_with_the_nonce_slot(self):
        self.assertEqual(m.EDGE_PAGE.count("__NONCE__"), 1)
        self.assertIn("WEBGL_debug_renderer_info", m.EDGE_PAGE)

    def test_superposition_click_derived_from_rectangle(self):
        script = m.render_check("superposition", superposition_run=True)
        self.assertIn("BoundingRectangle", script)
        self.assertIn("$rect.Left+$rect.Width*", script)
        self.assertIn("$rect.Top+$rect.Height*", script)
        self.assertIn("Remove-Item $source", script)

    def test_solidworks_default_launch_only(self):
        self.assertNotIn("OpenDoc6", m.render_check("solidworks", solidworks_export=False))
        self.assertIn("OpenDoc6", m.render_check("solidworks"))

    def test_solidworks_sample_comes_from_programdata_not_the_missing_samples_dir(self):
        script = m.render_check("solidworks")
        self.assertIn("C:\\ProgramData\\SolidWorks", script)
        self.assertNotIn("SOLIDWORKS\\samples", script)

    def edrawings(self, **changes):
        r = self.fixture(app="edrawings", kind="EDW", sample="kfsample.sldprt", accepted=True,
                         tab_name="kfsample.sldprt")
        for key in ("artifact", "modified", "bytes", "api_ok"):
            r.pop(key)
        r.update(changes)
        with tempfile.TemporaryDirectory() as td:
            (Path(td) / "result.json").write_text(json.dumps(r))
            return m.verdict("edrawings", "a" * 32, Path(td))

    def test_edrawings_tab_with_the_sample_name_passes_via_uia(self):
        row = self.edrawings()
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["evidence"], "UIA")

    def test_edrawings_licence_dialog_only_is_inconclusive(self):
        row = self.edrawings(tab_name="", accepted=False)
        self.assertEqual(m.app_classification(row)[0], "INCONCLUSIVE")

    def test_edrawings_generic_title_is_not_a_pass(self):
        row = self.edrawings(tab_name="eDrawings")
        self.assertEqual(m.app_classification(row)[0], "INCONCLUSIVE")

    def test_edrawings_wrong_sample_name_is_not_a_pass(self):
        self.assertFalse(self.edrawings(sample="other.sldprt")["ok"])

    def test_error_names_the_script_step_that_failed(self):
        with tempfile.TemporaryDirectory() as td:
            r = self.fixture(app="solidworks", kind="PROBE", error="Element not found", step="s5")
            (Path(td) / "result.json").write_text(json.dumps(r))
            row = m.verdict("solidworks", "a" * 32, Path(td))
        self.assertIn("script step s5", row["error"])

    def test_error_names_another_script_step(self):
        with tempfile.TemporaryDirectory() as td:
            r = self.fixture(app="edrawings", kind="PROBE", error="x", step="s9")
            (Path(td) / "result.json").write_text(json.dumps(r))
            self.assertIn("script step s9", m.verdict("edrawings", "a" * 32, Path(td))["error"])

    def test_error_without_step_has_no_step_suffix(self):
        with tempfile.TemporaryDirectory() as td:
            r = self.fixture(app="solidworks", kind="PROBE", error="boom")
            (Path(td) / "result.json").write_text(json.dumps(r))
            self.assertNotIn("script step", m.verdict("solidworks", "a" * 32, Path(td))["error"])

    def test_solidworks_script_marks_steps(self):
        self.assertIn("$r.step='open'", m.render_check("solidworks"))

    def test_solidworks_uses_late_bound_invokemember_not_the_powershell_adapter(self):
        # Measured 2026-11-14: the adapter path failed every call with TYPE_E_ELEMENTNOTFOUND.
        script = m.render_check("solidworks")
        self.assertIn("InvokeMember", script)
        self.assertIn("DispatchWrapper", script)
        self.assertNotIn("New-Object -ComObject", script)
        self.assertNotIn("$sw.OpenDoc6", script)

    def test_solidworks_byref_arguments_use_a_parameter_modifier(self):
        script = m.render_check("solidworks")
        self.assertIn("ParameterModifier", script)
        self.assertIn("@(4,5)", script)

    def test_edrawings_accept_uses_the_window_handle_not_invoke(self):
        # Measured 2026-11-14: ACCEPT is a Pane with no Invoke pattern; BM_CLICK on its handle worked.
        script = m.render_check("edrawings")
        self.assertIn("0xF5", script)
        self.assertIn("NativeWindowHandle", script)
        self.assertNotIn("InvokePattern", script)

    def test_edrawings_evidence_is_the_tab_item_not_the_window_title(self):
        script = m.render_check("edrawings")
        self.assertIn("ControlType.TabItem", script)
        self.assertNotIn("-match 'kfsample'", script)

    def test_edrawings_script_accepts_by_uia_selector_not_coordinates(self):
        script = m.render_check("edrawings")
        self.assertIn("-ceq 'ACCEPT'", script)
        self.assertIn("AutomationId -ceq '1'", script)
        self.assertNotIn("SetCursorPos", script)

    def resolve(self, **changes):
        wanted = "kf" + "a" * 8
        log = [
            "[0x1] | Main.GPUConfig | INFO  | 2026-11-12 18:10:20,984 | Selected compute API: CUDA",
            "[0x2] | IO | INFO  | 2026-11-12 18:10:22,185 | Nvidia GPU (0) is initialised as decoding and encoding device.",
            "[0x3] | UI.GLContext | INFO  | 2026-11-12 18:10:24,156 | Initialized OpenGL 4.5 (requested 4.5) on device 'NVIDIA Corporation NVIDIA GeForce RTX 4070/PCIe/SSE2'",
        ]
        r = self.fixture(app="resolve", kind="RESOLVE", project_name=wanted,
                         window_name="DaVinci Resolve - " + wanted, log_lines=log, stage="main window")
        for key in ("artifact", "modified", "bytes", "api_ok"):
            r.pop(key)
        r.update(changes)
        with tempfile.TemporaryDirectory() as td:
            (Path(td) / "result.json").write_text(json.dumps(r))
            return m.verdict("resolve", "a" * 32, Path(td))

    def test_resolve_project_window_and_gpu_log_pass(self):
        row = self.resolve()
        self.assertTrue(row["ok"], row)
        self.assertIn("no render or benchmark was run", row["signal"])

    def test_resolve_window_without_the_projects_name_is_inconclusive(self):
        row = self.resolve(window_name="DaVinci Resolve - New Project 1")
        self.assertEqual(m.app_classification(row)[0], "INCONCLUSIVE")

    def test_resolve_project_name_mismatch_is_inconclusive(self):
        row = self.resolve(project_name="kfbbbbbbbb")
        self.assertEqual(m.app_classification(row)[0], "INCONCLUSIVE")

    def test_resolve_without_cuda_line_is_inconclusive(self):
        log = [x for x in self.resolve_log() if "compute API" not in x]
        self.assertIn("CUDA", self.resolve(log_lines=log)["error"])

    def test_resolve_without_nvenc_line_is_inconclusive(self):
        log = [x for x in self.resolve_log() if "decoding and encoding" not in x]
        self.assertIn("decode/encode", self.resolve(log_lines=log)["error"])

    def test_resolve_without_opengl_line_is_inconclusive(self):
        log = [x for x in self.resolve_log() if "OpenGL" not in x]
        self.assertIn("OpenGL", self.resolve(log_lines=log)["error"])

    def test_resolve_software_opengl_is_inconclusive(self):
        log = [x.replace("NVIDIA Corporation NVIDIA GeForce RTX 4070", "Microsoft Corporation GDI Generic") for x in self.resolve_log()]
        self.assertIn("OpenGL", self.resolve(log_lines=log)["error"])

    def test_resolve_nvidia_named_llvmpipe_opengl_is_inconclusive(self):
        log = [x.replace("NVIDIA GeForce RTX 4070", "NVIDIA llvmpipe") for x in self.resolve_log()]
        self.assertIn("OpenGL", self.resolve(log_lines=log)["error"])

    def test_resolve_empty_log_names_cuda_missing(self):
        self.assertIn("CUDA", self.resolve(log_lines=[])["error"])

    def test_resolve_empty_log_names_encode_device_missing(self):
        self.assertIn("decode/encode", self.resolve(log_lines=[])["error"])

    def resolve_log(self):
        return list(self._resolve_log_lines)

    _resolve_log_lines = [
        "[0x1] | Main.GPUConfig | INFO  | 2026-11-12 18:10:20,984 | Selected compute API: CUDA",
        "[0x2] | IO | INFO  | 2026-11-12 18:10:22,185 | Nvidia GPU (0) is initialised as decoding and encoding device.",
        "[0x3] | UI.GLContext | INFO  | 2026-11-12 18:10:24,156 | Initialized OpenGL 4.5 (requested 4.5) on device 'NVIDIA Corporation NVIDIA GeForce RTX 4070/PCIe/SSE2'",
    ]

    def test_resolve_script_finds_ui_by_automationid_and_invokes(self):
        script = m.render_check("resolve", run_id="a" * 32)
        for needle in ("UiProjectManagerDialog", "btnAddProject", "InvokePattern", "UiMainWindow"):
            self.assertIn(needle, script)
        self.assertNotIn("SetCursorPos", script)
        self.assertNotIn("mouse_event", script)

    def test_resolve_script_types_only_the_project_name(self):
        script = m.render_check("resolve", run_id="a" * 32)
        self.assertEqual(script.count("SendWait("), 3)
        self.assertIn("SendWait($name)", script)

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
        ("tdm-nofile", 'if not r.get("result_file"):', "if False:"),
        ("tdm-progress", 'if r.get("progress_seen") is True:', "if True:"),
        ("tdm-stale", "if type(modified) not in (int, float) or not start < modified <= end:\n                raise ValueError(\"Stale result file\")", "pass"),
        ("tdm-name", 'if not re.fullmatch(r"[A-Za-z0-9 ._()\\-]{1,120}\\.3dmark-result", str(r["result_file"])):', "if False:"),
        ("tdm-window", "if not (run_start <= began < finished <= run_end):", "if False:"),
        ("tdm-status", 'if not codes or any(c != "0" for c in codes):', "if False:"),
        ("tdm-score", "if not total or min(total) <= 0 or not graphics or min(graphics) <= 0:", "if False:"),
        ("tdm-fps", "if not fps or min(fps) <= 0:", "if False:"),
        ("tdm-gpu", "if not gpus:", "if False:"),
        ("tdm-software", "if any(SOFTWARE_RENDERERS.search(name) for name, _ in gpus):", "if False:"),
        ("tdm-entity", 'if re.search(rb"<!(DOCTYPE|ENTITY)", raw, re.I):', "if False:"),
        ("tdm-member", "if info.filename not in TDM_MEMBERS or info.file_size > TDM_MAX_MEMBER:", "if False:"),
        ("run-replace", 'kw.setdefault("errors", "replace")', "pass"),
        ("res-name", 'if r.get("project_name") != wanted or wanted not in str(r.get("window_name", "")):', "if False:"),
        ("res-cuda", 'if "Selected compute API: CUDA" not in blob:', "if False:"),
        ("res-enc", 'if "initialised as decoding and encoding device" not in blob:', "if False:"),
        ("res-gl", 'if not gl or "nvidia" not in gl[1].lower() or SOFTWARE_RENDERERS.search(gl[1]):', "if False:"),
        ("edw-title", 'tab.lower() != "kfsample.sldprt"', "False"),
        ("edw-sample", 'r.get("sample") != "kfsample.sldprt" or', "False and"),
        ("hostlog-status", "if STATUS_LINE.match(line):", "if False:"),
        ("hostlog-nonzero", "return bool(STATUS_NONZERO.search(line))", "return False"),
        ("hostlog-limit", "[-limit:]", "[:]"),
        ("hostlog-trunc", "ln[:400]", "ln"),
        ("error-step", 'if r.get("step") else ""', 'if False else ""'),
        ("wukong-crash", 'if crash:', "if False:"),
        ("wukong-kernel", 'if str(e.get("code")) in GPU_KERNEL_EVENTS', "if False and str(e.get(\"code\")) in GPU_KERNEL_EVENTS"),
        ("wukong-start", 'if r.get("shipping_seen") is not True:', "if False:"),
        ("wukong-apperr", 'if r.get("exited") is True and r.get("app_errors"):', "if False:"),
        ("task-hidden", "-NoProfile -WindowStyle Hidden -ExecutionPolicy", "-NoProfile -ExecutionPolicy"),
        ("edge-software", 'if SOFTWARE_RENDERERS.search(renderer):', "if False:"),
        ("edge-nvidia", 'if "nvidia" not in renderer.lower():', "if False:"),
        ("edge-frames", "if frames <= 0:", "if False:"),
        ("edge-nonce", "if ok[1] != run_id:", "if False:"),
        ("edge-window", 'if r.get("launched") is not True:', "if False:"),
        ("sp-window", "if interval > end - start:", "if False:"),
        ("sp-fatal", 'if facts["fatal"]:', "if False:"),
        ("sp-software", "if SOFTWARE_RENDERERS.search(gpu + \" \" + renderer):", "if False:"),
        ("sp-nvidia", '"nvidia" not in gpu.lower() or not gpu.startswith(renderer)', "not gpu.startswith(renderer)"),
        ("sp-renderer-match", '"nvidia" not in gpu.lower() or not gpu.startswith(renderer)', '"nvidia" not in gpu.lower()'),
        ("sp-names-gpu", "if not gpu or not renderer:", "if False:"),
        ("sp-positive-interval", "if not interval or interval <= 0:", "if False:"),
        ("sp-wrap", "% 86400", ""),
        ("sp-screen-fps", "if not (0 < avg and 0 <= lo <= avg <= hi and score > 0):", "if False:"),
    ]
    failed = False
    for name, old, new in cases:
        expected_count = 2 if name in ("length", "task-cleanup", "freshness") else 1
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
