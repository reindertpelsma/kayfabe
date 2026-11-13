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

SRC = Path(__file__).resolve().parents[2] / "scripts/bench/windows/propimage_apps.py"
spec = importlib.util.spec_from_file_location("fixture_propimage_apps", SRC)
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
            row = m.run_check("solidworks", {}, Path(td), runner=runner, sleep=lambda _: None)
            self.assertTrue(row["ok"])
            self.assertEqual(row["evidence"], "FILE")
            self.assertTrue((Path(td) / "solidworks" / ("a" * 32) / "result.json").exists())
        self.assertTrue(
            any("Unregister-ScheduledTask" in (k.get("input", "") or "") for _, k in calls)
        )

    def test_transfer_failure_does_not_pass(self):
        def runner(argv, **kw):
            return SimpleNamespace(returncode=1, stdout="", stderr="failed")

        with tempfile.TemporaryDirectory() as td:
            row = m.run_check("solidworks", {}, Path(td), runner=runner)
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


def mutants():
    """In-memory, authored-source mutations; two independent test methods per rule."""
    global m
    original = m
    source = SRC.read_text()
    cases = [
        ("run-binding", 'r.get("run_id") != run_id', "False"),
        ("app-binding", 'r.get("app") != name', "False"),
        ("freshness", "not start < modified <= end", "False"),
        ("length", 'len(raw) != r.get("bytes")', "False"),
        ("api-success", 'r.get("api_ok") is not True', "False"),
        ("step-structure", 'raise ValueError("Invalid STEP exchange structure")', "pass"),
        ("progress-transition", 'record.get("progress") == running', "True"),
        ("prior-state", 'record.get("before") not in (None, "", running, finished)', "True"),
    ]
    failed = False
    for name, old, new in cases:
        if source.count(old) != 1:
            raise AssertionError((name, source.count(old)))
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "mutant_propimage_apps.py"
            path.write_text(source.replace(old, new))
            mutation_spec = importlib.util.spec_from_file_location("mutant_propimage_apps", path)
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
