"""GPU-free contract tests. Falsifier: an invalid plan runs, RETRY passes, or a child survives.

The legacy stage fixture is byte-for-byte apps_gate.sh at 50640c605, retained only as
an equivalence oracle; no production entry point executes it.
"""

import io
import json
import os
import shutil
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from kfci.assignment import AssignmentError, load, validate
from kfci.catalog import Catalog, CatalogError
from kfci.proc import run
from kfci.runner import Runner
from kfci.step import RetryClass, Step, StepResult, Verdict
from kfci.testing import FakeClock, FakeStep


def assignment(steps, **kw):
    return dict(
        version=1,
        id="fake_box",
        budget_seconds=600,
        steps=[{"id": s} for s in steps],
        **kw
    )


class Assignments(unittest.TestCase):
    def test_file_and_dict_have_the_same_contract(self):
        value = assignment(["one"])
        self.assertEqual(validate(value), value)
        self.assertEqual(load(io.StringIO(json.dumps(value))), value)

    def test_missing_required_fields_are_refused(self):
        for key in ("version", "id", "budget_seconds", "steps"):
            value = assignment(["one"])
            del value[key]
            with self.subTest(key), self.assertRaises(AssignmentError):
                validate(value)

    def test_invalid_fields_are_refused(self):
        for key, val in (
            ("version", 2),
            ("version", True),
            ("id", "a\nb"),
            ("budget_seconds", 0),
            ("budget_seconds", True),
            ("budget_seconds", float("nan")),
            ("budget_seconds", 10**400),
            ("steps", []),
            ("steps", [{"id": "one"}, {"id": "one"}]),
            ("steps", [{"id": "one", "command": ["echo"]}]),
            ("extra", 1),
        ):
            value = assignment(["one"])
            value[key] = val
            with self.subTest(key, val=val), self.assertRaises(AssignmentError):
                validate(value)

    def test_another_invalid_budget_or_identifier_is_refused(self):
        for key, val in (
            ("budget_seconds", -1),
            ("budget_seconds", float("inf")),
            ("id", "../box"),
            ("steps", [{"id": "a b"}]),
        ):
            value = assignment(["one"])
            value[key] = val
            with self.subTest(key, val=val), self.assertRaises(AssignmentError):
                validate(value)

    def test_reserve_inputs_and_parameters_are_checked(self):
        for patch in (
            {"reserve_seconds": -1},
            {"reserve_seconds": 600},
            {"inputs": ["x", "x"]},
            {"steps": [{"id": "one", "parameters": []}]},
        ):
            value = assignment(["one"])
            value.update(patch)
            with self.subTest(patch), self.assertRaises(AssignmentError):
                validate(value)

    def test_duplicate_json_keys_are_refused(self):
        with self.assertRaises(AssignmentError):
            raw = json.dumps(assignment(["one"]))
            load(io.StringIO(raw.replace('"version": 1', '"version": 1, "version": 1')))


class Dependencies(unittest.TestCase):
    def setUp(self):
        self.cat = Catalog()
        self.cat.register(FakeStep("prepare", produces=("image",)))
        self.cat.register(FakeStep("check", needs=("image",), produces=("checked",)))
        self.cat.register(FakeStep("finish", needs=("checked",)))

    def test_local_resolver_adds_prerequisites_in_order(self):
        self.assertEqual(self.cat.resolve(["finish"]), ["prepare", "check", "finish"])

    def test_local_resolver_reuses_available_input(self):
        self.assertEqual(
            self.cat.resolve(["finish"], inputs=["image"]), ["check", "finish"]
        )

    def test_worker_order_is_validated_without_replanning(self):
        self.assertEqual(
            [s.id for s in self.cat.ordered(["prepare", "check"])], ["prepare", "check"]
        )
        with self.assertRaisesRegex(CatalogError, "image"):
            self.cat.ordered(["check", "prepare"])

    def test_unknown_and_duplicate_steps_are_errors(self):
        with self.assertRaises(CatalogError):
            self.cat.resolve(["absent"])
        with self.assertRaises(CatalogError):
            self.cat.register(FakeStep("prepare"))

    def test_cycles_and_missing_products_are_errors(self):
        self.cat.register(FakeStep("cycle", needs=("loop",), produces=("loop",)))
        with self.assertRaisesRegex(CatalogError, "cycle"):
            self.cat.resolve(["cycle"])
        self.cat.register(FakeStep("missing", needs=("absent",)))
        with self.assertRaisesRegex(CatalogError, "no producer for absent"):
            self.cat.resolve(["missing"])

    def test_ambiguous_product_requires_an_explicit_producer(self):
        self.cat.register(FakeStep("other", produces=("image",)))
        with self.assertRaisesRegex(CatalogError, "ambiguous"):
            self.cat.resolve(["check"])
        self.assertEqual(self.cat.resolve(["prepare", "check"]), ["prepare", "check"])


class Execution(unittest.TestCase):
    def execute(self, steps, plan=None, clock=None):
        cat = Catalog()
        for step in steps:
            cat.register(step)
        output = io.StringIO()
        summary = Runner(cat, output=output, clock=clock or FakeClock()).run(
            plan or assignment([s.id for s in steps])
        )
        return summary, output.getvalue()

    def test_success_writes_rows_and_summary_in_order(self):
        result, out = self.execute([FakeStep("one"), FakeStep("two")])
        self.assertEqual(result["verdict"], "PASS")
        self.assertEqual(result["rc"], 0)
        self.assertEqual([r["id"] for r in result["steps"]], ["one", "two"])
        self.assertLess(
            out.index("KFCI_STEP_START id=one"), out.index("KFCI_ROW id=one")
        )
        self.assertLess(
            out.index("KFCI_ROW id=one"), out.index("KFCI_STEP_EXIT id=one rc=0")
        )
        self.assertLess(
            out.index("KFCI_STEP_EXIT id=one"), out.index("KFCI_STEP_START id=two")
        )
        self.assertTrue(out.endswith("KFCI_EXIT rc=0\n"))

    def test_retry_is_not_pass_and_stops(self):
        first, second = FakeStep(
            "one", result=StepResult.retry(RetryClass.MEDIA)
        ), FakeStep("two")
        result, out = self.execute([first, second])
        self.assertEqual((result["verdict"], result["rc"]), ("RETRY", 3))
        self.assertIn("class=MEDIA", out)
        self.assertEqual(second.calls, [])

    def test_fail_stops_the_following_step(self):
        first, second = FakeStep("one", result=StepResult(Verdict.FAIL)), FakeStep(
            "two"
        )
        result, _ = self.execute([first, second])
        self.assertEqual((result["verdict"], result["rc"]), ("FAIL", 1))
        self.assertEqual(second.calls, [])

    def test_time_guard_never_starts_a_step_without_its_estimate(self):
        step = FakeStep("one", estimated_minutes=11)
        result, out = self.execute([step])
        self.assertEqual((result["verdict"], result["rc"]), ("RETRY", 3))
        self.assertEqual(step.calls, [])
        self.assertIn("class=TIME", out)

    def test_tail_reserve_is_not_spent(self):
        step = FakeStep("one", estimated_minutes=10)
        plan = assignment(["one"], reserve_seconds=1)
        result, _ = self.execute([step], plan)
        self.assertEqual(result["verdict"], "RETRY")
        self.assertEqual(step.calls, [])

    def test_budget_is_rechecked_after_each_step(self):
        clock = FakeClock()
        one = FakeStep("one", action=lambda ctx: clock.advance(550))
        two = FakeStep("two", estimated_minutes=1)
        result, _ = self.execute([one, two], clock=clock)
        self.assertEqual(result["verdict"], "RETRY")
        self.assertEqual(two.calls, [])

    def test_an_overdue_pass_is_retry_time(self):
        clock = FakeClock()
        step = FakeStep("one", timeout_seconds=2, action=lambda ctx: clock.advance(3))
        result, _ = self.execute([step], clock=clock)
        self.assertEqual(
            (result["verdict"], result["steps"][0]["retry_class"]), ("RETRY", "TIME")
        )

    def test_step_exception_writes_exit_and_is_retry_operator(self):
        def broken(ctx):
            raise RuntimeError("private diagnostic")

        result, out = self.execute([FakeStep("one", action=broken)])
        self.assertEqual(result["verdict"], "RETRY")
        self.assertIn("KFCI_STEP_EXIT id=one rc=3", out)
        self.assertNotIn("private diagnostic", out)

    def test_invalid_order_is_refused_before_any_work(self):
        one = FakeStep("one", needs=("image",))
        with self.assertRaises(CatalogError):
            self.execute([one])
        self.assertEqual(one.calls, [])

    def test_retry_requires_a_class_and_pass_cannot_have_one(self):
        with self.assertRaises(ValueError):
            StepResult(Verdict.RETRY)
        with self.assertRaises(ValueError):
            StepResult(Verdict.PASS, RetryClass.TIME)

    def test_context_rows_are_visible_before_completion(self):
        cat, output = Catalog(), io.StringIO()

        def emit(ctx):
            ctx.emit("APP_ROW name=fake os=linux result=PASS secs=1 guest_xid=0 note=-")
            self.assertIn("APP_ROW", output.getvalue())
            self.assertNotIn("KFCI_STEP_EXIT", output.getvalue())

        cat.register(FakeStep("one", action=emit))
        Runner(cat, output=output, clock=FakeClock()).run(assignment(["one"]))


class Processes(unittest.TestCase):
    def python(self, body, **kw):
        return run(
            [sys.executable, "-I", "-c", body], timeout=kw.pop("timeout", 3), **kw
        )

    def test_argument_list_preserves_metacharacters(self):
        result = run(
            [
                sys.executable,
                "-I",
                "-c",
                "import sys;print(sys.argv[1])",
                "$(false); a b",
            ],
            timeout=3,
        )
        self.assertEqual(result.output, "$(false); a b\n")
        self.assertEqual(result.returncode, 0)

    def test_string_command_and_nonstring_arguments_are_refused(self):
        for argv in ("echo unsafe", ["echo", 7]):
            with self.subTest(argv), self.assertRaises((ValueError, TypeError)):
                run(argv, timeout=3)

    def test_output_is_bounded_even_without_newlines(self):
        result = self.python("import os;os.write(1,b'x'*2000000)", capture_limit=1234)
        self.assertEqual(len(result.output), 1234)
        self.assertTrue(result.truncated)

    def test_stderr_is_also_bounded(self):
        result = self.python("import os;os.write(2,b'y'*2000000)", capture_limit=1000)
        self.assertEqual(len(result.output), 1000)
        self.assertTrue(result.truncated)

    def test_environment_is_allowlisted(self):
        result = self.python(
            "import os;print(os.getenv('UNLISTED'));print(os.getenv('LANG'))",
            env={"UNLISTED": "secret", "LANG": "C"},
        )
        self.assertEqual(result.output, "None\nC\n")

    def test_parent_environment_is_not_implicitly_forwarded(self):
        from unittest.mock import patch

        with patch.dict(os.environ, {"UNLISTED": "secret"}):
            result = self.python("import os;print(os.getenv('UNLISTED'))")
        self.assertEqual(result.output, "None\n")

    def test_timeout_is_not_success(self):
        result = self.python(
            "import time;time.sleep(30)", timeout=0.1, grace_seconds=0.1
        )
        self.assertTrue(result.timed_out)
        self.assertEqual(result.returncode, 124)

    def test_process_group_kills_child_ignoring_term(self):
        started = time.monotonic()
        result = self.python(
            "import subprocess,sys,time;subprocess.Popen([sys.executable,'-c','import signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(30)']);print('ready',flush=True);time.sleep(30)",
            timeout=0.3,
            grace_seconds=0.1,
        )
        self.assertTrue(result.timed_out)
        self.assertLess(time.monotonic() - started, 3)

    def test_child_is_killed_when_parent_exits_first(self):
        started = time.monotonic()
        result = self.python(
            "import subprocess,sys;subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)']);print('parent-done')",
            grace_seconds=0.1,
        )
        self.assertEqual(result.returncode, 0)
        self.assertLess(time.monotonic() - started, 2)

    def test_tick_exception_cleans_up_owned_process(self):
        def tick():
            raise RuntimeError("callback")

        started = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, "callback"):
            self.python("import time;time.sleep(30)", on_tick=tick, grace_seconds=0.1)
        self.assertLess(time.monotonic() - started, 2)


class LaunchModes(unittest.TestCase):
    def invoke(self, mode, plan, env):
        command = [sys.executable, "-I", str(SCRIPTS / "kfci/main.py"), mode]
        with tempfile.TemporaryDirectory() as d:
            if mode == "direct":
                path = Path(d) / "assignment.json"
                path.write_text(json.dumps(plan))
                command.append(str(path))
                raw = None
            else:
                raw = json.dumps(plan) + "\n"
                if mode == "daemon":
                    raw += json.dumps(dict(plan, id="second")) + "\n"
            return subprocess.run(
                command, input=raw, capture_output=True, text=True, timeout=10, env=env
            )

    def test_three_launch_modes_share_the_same_stage_result(self):
        with tempfile.TemporaryDirectory() as d:
            plan = assignment(["apps_gate"], inputs=["linux_apps_ready"])
            plan["budget_seconds"] = 4000
            plan["steps"][0]["parameters"] = dict(tag="fake", prov=d)
            env = {
                "PATH": os.environ["PATH"],
                "KF_APPS_LIST": str(Path(d) / "missing"),
                "PYTHONPATH": "/does-not-exist",
            }
            outputs = [
                self.invoke(m, plan, env) for m in ("direct", "worker", "daemon")
            ]
        self.assertEqual([p.returncode for p in outputs], [3, 3, 0])
        self.assertEqual(outputs[0].stdout, outputs[1].stdout)
        for out in outputs:
            self.assertIn("APPS_STAGE_RETRY class=NO_LIST", out.stdout)
            self.assertIn(
                "KFCI_ROW id=apps_gate result=RETRY class=NO_LIST", out.stdout
            )
        self.assertEqual(outputs[2].stdout.count("KFCI_RESULT "), 2)

    def test_invalid_worker_and_direct_assignments_never_start(self):
        for mode in ("direct", "worker", "daemon"):
            with self.subTest(mode):
                out = self.invoke(
                    mode, {"command": "forbidden"}, {"PATH": os.environ["PATH"]}
                )
                self.assertEqual(out.returncode, 2)
                self.assertNotIn("KFCI_STEP_START", out.stdout)

    def test_unknown_worker_step_is_refused(self):
        out = self.invoke(
            "worker", assignment(["arbitrary_command"]), {"PATH": os.environ["PATH"]}
        )
        self.assertEqual(out.returncode, 2)
        self.assertNotIn("KFCI_STEP_START", out.stdout)


class AppsRules(unittest.TestCase):
    def test_failed_app_beats_missing_rows(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(1, 1, 1, 3, 0).verdict, Verdict.FAIL)

    def test_failed_app_beats_timeout(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(0, 1, 1, 2, 124).verdict, Verdict.FAIL)

    def test_missing_rows_without_timeout_retry_box(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(0, 0, 2, 2, 0), StepResult.retry(RetryClass.BOX))

    def test_one_missing_row_cannot_pass(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(1, 0, 1, 2, 0), StepResult.retry(RetryClass.BOX))

    def test_timeout_retries_time(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(0, 0, 2, 2, 124), StepResult.retry(RetryClass.TIME))

    def test_forced_kill_retries_time(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(1, 0, 1, 2, 137), StepResult.retry(RetryClass.TIME))

    def test_nonzero_matrix_cannot_pass_complete_rows(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(2, 0, 0, 2, 1), StepResult.retry(RetryClass.OPERATOR))

    def test_dead_matrix_cannot_pass_complete_rows(self):
        from kfci.apps_gate import decide

        self.assertEqual(decide(1, 0, 0, 1, 137), StepResult.retry(RetryClass.OPERATOR))

    def test_notrun_row_is_not_failure(self):
        from kfci.apps_gate import grade_row

        row = "APPRES side=guest app=a verdict=NOTRUN rc=- secs=0 note=absent boot=t"
        self.assertEqual(grade_row("a", row)[0], "NOT_RUN")

    def test_boot_fail_row_is_not_failure(self):
        from kfci.apps_gate import grade_row

        row = "APPRES side=guest app=a verdict=BOOT_FAIL rc=- secs=0 note=boot boot=t"
        self.assertEqual(grade_row("a", row)[0], "NOT_RUN")

    def test_isolated_last_row_is_authoritative(self):
        from kfci.apps_gate import last_row

        with tempfile.TemporaryDirectory() as d:
            isolated, batched = Path(d) / "isolated", Path(d) / "batched"
            isolated.write_text(
                "APPRES side=guest app=a verdict=FAIL \nAPPRES side=guest app=a verdict=PASS \n"
            )
            batched.write_text("APPRES side=guest app=a verdict=FAIL \n")
            self.assertIn("verdict=PASS", last_row([isolated, batched], "a"))

    def test_isolated_failure_is_authoritative(self):
        from kfci.apps_gate import last_row

        with tempfile.TemporaryDirectory() as d:
            isolated, batched = Path(d) / "isolated", Path(d) / "batched"
            isolated.write_text("APPRES side=guest app=a verdict=FAIL \n")
            batched.write_text("APPRES side=guest app=a verdict=PASS \n")
            self.assertIn("verdict=FAIL", last_row([isolated, batched], "a"))

    def test_note_is_limited_to_120_characters(self):
        from kfci.apps_gate import grade_row

        row = (
            "APPRES side=guest app=a verdict=PASS rc=0 secs=1 note="
            + "x" * 150
            + " boot=t"
        )
        self.assertEqual(len(grade_row("a", row)[1].split("note=", 1)[1]), 120)

    def test_progress_and_notes_cannot_grow_to_guest_line_size(self):
        from kfci.apps_gate import grade_row

        row = (
            "APPRES side=guest app=a verdict=PASS rc=0 secs=1 note="
            + "y" * 500
            + " boot=t"
        )
        self.assertEqual(len(grade_row("a", row)[1].split("note=", 1)[1]), 120)


class Contracts(unittest.TestCase):

    def test_guest_diagnostic_environment_is_kept(self):
        from kfci.apps_gate import legacy_allowlist

        env = {"KF3_PT_STALL_SNAPSHOT": "1"}
        result = run(
            [
                sys.executable,
                "-I",
                "-c",
                "import os;print(os.getenv('KF3_PT_STALL_SNAPSHOT'))",
            ],
            timeout=3,
            env=env,
            allow_env=legacy_allowlist(env),
        )
        self.assertEqual(result.output, "1\n")

    def test_product_build_environment_is_kept(self):
        from kfci.apps_gate import legacy_allowlist

        env = {"KAYFABE_ISOLATE_IMAGE_STUB": "0"}
        result = run(
            [
                sys.executable,
                "-I",
                "-c",
                "import os;print(os.getenv('KAYFABE_ISOLATE_IMAGE_STUB'))",
            ],
            timeout=3,
            env=env,
            allow_env=legacy_allowlist(env),
        )
        self.assertEqual(result.output, "0\n")

    def test_unrelated_environment_is_not_kept_by_the_stage(self):
        from kfci.apps_gate import legacy_allowlist

        env = {"UNLISTED_TOKEN": "sensitive"}
        result = run(
            [
                sys.executable,
                "-I",
                "-c",
                "import os;print(os.getenv('UNLISTED_TOKEN'))",
            ],
            timeout=3,
            env=env,
            allow_env=legacy_allowlist(env),
        )
        self.assertEqual(result.output, "None\n")

    def test_missing_version_is_a_validation_error(self):
        plan = assignment(["one"])
        del plan["version"]
        with self.assertRaises(AssignmentError):
            validate(plan)

    def test_nested_duplicate_json_key_is_refused(self):
        raw = json.dumps(assignment(["one"]))
        raw = raw.replace('"id": "one"', '"id": "one", "id": "two"')
        with self.assertRaises(AssignmentError):
            load(io.StringIO(raw))

    def test_duplicate_step_id_is_refused(self):
        with self.assertRaises(AssignmentError):
            validate(assignment(["one", "one"]))

    def test_unknown_assignment_field_cannot_supply_commands(self):
        plan = assignment(["one"])
        plan["command"] = ["false"]
        with self.assertRaises(AssignmentError):
            validate(plan)

    def test_second_overdue_step_cannot_pass(self):
        clock, cat = FakeClock(), Catalog()
        cat.register(FakeStep("one"))
        cat.register(
            FakeStep("two", timeout_seconds=1, action=lambda ctx: clock.advance(2))
        )
        result = Runner(cat, clock=clock, output=io.StringIO()).run(
            assignment(["one", "two"])
        )
        self.assertEqual(result["verdict"], "RETRY")
        self.assertEqual(result["steps"][1]["retry_class"], "TIME")

    def test_another_ambiguous_dependency_is_an_error(self):
        cat = Catalog()
        for name in ("first", "second"):
            cat.register(FakeStep(name, produces=("image",)))
        cat.register(FakeStep("consumer", needs=("image",)))
        with self.assertRaisesRegex(CatalogError, "ambiguous"):
            cat.resolve(["consumer"])

    def test_mutual_dependency_cycle_has_a_clear_error(self):
        cat = Catalog()
        cat.register(FakeStep("one", needs=("b",), produces=("a",)))
        cat.register(FakeStep("two", needs=("a",), produces=("b",)))
        with self.assertRaisesRegex(CatalogError, "cycle"):
            cat.resolve(["one"])

    def test_unavailable_product_has_a_clear_error(self):
        cat = Catalog()
        cat.register(FakeStep("one", needs=("missing",)))
        with self.assertRaisesRegex(CatalogError, "no producer for missing"):
            cat.resolve(["one"])

    def test_zero_step_timeout_is_refused(self):
        with self.assertRaises(CatalogError):
            Catalog().register(FakeStep("one", timeout_seconds=0))

    def test_infinite_step_estimate_is_refused(self):
        with self.assertRaises(CatalogError):
            Catalog().register(FakeStep("one", estimated_minutes=float("inf")))

    def test_child_has_its_own_session(self):
        result = run(
            [sys.executable, "-I", "-c", "import os;print(os.getsid(0)==os.getpid())"],
            timeout=3,
        )
        self.assertEqual(result.output, "True\n")

    def test_child_has_its_own_process_group(self):
        result = run(
            [sys.executable, "-I", "-c", "import os;print(os.getpgrp()==os.getpid())"],
            timeout=3,
        )
        self.assertEqual(result.output, "True\n")

    def test_invalid_step_result_cannot_pass(self):
        cat = Catalog()
        step = FakeStep("one")
        step.result = "PASS"
        cat.register(step)
        result = Runner(cat, output=io.StringIO(), clock=FakeClock()).run(
            assignment(["one"])
        )
        self.assertEqual(result["verdict"], "RETRY")

    def test_unknown_retry_class_is_rejected(self):
        with self.assertRaises(ValueError):
            StepResult(Verdict.RETRY, "MEDIA")

    def test_retry_rc_and_class_are_consistent(self):
        for cls in (RetryClass.TIME, RetryClass.OPERATOR):
            self.assertEqual(StepResult.retry(cls).rc, 3)

    def test_another_timeout_cannot_be_success(self):
        result = run(
            [sys.executable, "-I", "-c", "import time;time.sleep(10)"],
            timeout=0.1,
            grace_seconds=0.1,
        )
        self.assertTrue(result.timed_out)
        self.assertNotEqual(result.returncode, 0)

    def test_second_step_cannot_spend_reserved_tail(self):
        clock, cat = FakeClock(), Catalog()
        cat.register(FakeStep("one", action=lambda ctx: clock.advance(500)))
        later = cat.register(FakeStep("two", estimated_minutes=1))
        plan = assignment(["one", "two"], reserve_seconds=50)
        result = Runner(cat, clock=clock, output=io.StringIO()).run(plan)
        self.assertEqual(result["verdict"], "RETRY")
        self.assertEqual(later.calls, [])


class Cancellation(unittest.TestCase):
    def test_cancelled_stage_cleans_its_matrix_and_writes_exit(self):
        import signal

        self.cancel(signal.SIGTERM)

    def test_interrupted_stage_cleans_its_matrix_and_writes_exit(self):
        import signal

        self.cancel(signal.SIGINT)

    def cancel(self, interrupt):
        import time
        import signal

        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            # This lifecycle test supplies a fake name enumerator as well as a
            # fake matrix. Gate-list parity is exercised by the migration suite.
            shutil.copytree(
                SCRIPTS / "kfci",
                root / "scripts/kfci",
                ignore=shutil.ignore_patterns("__pycache__"),
            )
            (root / "scripts/apps").mkdir()
            (root / "scripts/apps/run_apps.sh").write_text(
                "#!/bin/bash\nprintf 'vectorAdd \\n'\n"
            )
            (root / "list").write_text("vectorAdd\n")
            (root / "apps").mkdir()
            (root / "apps/bundle.tgz").write_text("x")
            (root / "apps/PROVISIONED").write_text("x")
            marker = root / "child-pid"
            (root / "matrix").write_text(
                '#!/bin/bash\necho $$ > "' + str(marker) + '"\nsleep 60\n'
            )
            env = dict(
                PATH=os.environ["PATH"],
                KF_APPS_LIST=str(root / "list"),
                KF_APPS_ROOT=str(root / "apps"),
                KF_APPS_MATRIX=str(root / "matrix"),
                KF_APPS_POLL_S="0.05",
            )
            child = subprocess.Popen(
                [
                    sys.executable,
                    "-I",
                    str(root / "scripts/kfci/main.py"),
                    "apps-gate",
                    "fake",
                    str(root / "prov"),
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=env,
            )
            pgid = None
            try:
                until = time.monotonic() + 5
                while (
                    not marker.exists() or marker.stat().st_size == 0
                ) and time.monotonic() < until:
                    time.sleep(0.02)
                self.assertTrue(marker.exists(), "fake matrix did not start")
                pid = int(marker.read_text())
                pgid = os.getpgid(pid)
                self.assertNotEqual(
                    pgid, os.getpgrp(), "matrix must have its own process group"
                )
                child.send_signal(interrupt)
                stdout, stderr = child.communicate(timeout=5)
                self.assertEqual(child.returncode, 3, stderr)
                self.assertTrue(
                    stdout.endswith(
                        "APPS_STAGE_RETRY class=OPERATOR\nAPPS_STAGE_EXIT rc=3\n"
                    ),
                    stdout,
                )
                stat = Path("/proc") / str(pid) / "stat"
                until = time.monotonic() + 1
                while (
                    stat.exists()
                    and stat.read_text().split()[2] != "Z"
                    and time.monotonic() < until
                ):
                    time.sleep(0.01)
                self.assertTrue(
                    not stat.exists() or stat.read_text().split()[2] == "Z",
                    "matrix survived cancellation",
                )
            finally:
                if pgid is not None and pgid != os.getpgrp():
                    try:
                        os.killpg(pgid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if child.poll() is None:
                    child.kill()
                    child.communicate(timeout=5)


class ImageCopy(unittest.TestCase):
    def copy(self, mode, mask, expected):
        from kfci.apps_gate import copy_image

        with tempfile.TemporaryDirectory() as d:
            source, destination = Path(d) / "source", Path(d) / "destination"
            source.write_bytes(b"fake image bytes")
            source.chmod(mode)
            original = os.umask(mask)
            try:
                copy_image(source, destination)
            finally:
                os.umask(original)
            self.assertEqual(destination.read_bytes(), b"fake image bytes")
            self.assertEqual(destination.stat().st_mode & 0o777, expected)

    def test_private_source_stays_private(self):
        self.copy(0o600, 0o022, 0o600)

    def test_group_readable_source_keeps_its_mode(self):
        self.copy(0o640, 0o022, 0o640)

    def test_strict_umask_still_applies(self):
        self.copy(0o644, 0o077, 0o600)


if __name__ == "__main__":
    unittest.main()
