"""2026-11-16: the optional `kfml` payload stage (scripts/ci/kfml_stage.py, vast_payload.py, vast_report.py), GPU-free.

FALSIFIER (written before the code): the stage is in a default plan, or cannot be selected with --only kfml in the middle and large
tiers (or can be in small/adhoc); a box on which no pinned wheel flavour fits is a FAIL (it must be a skipped NOT_RUN row); a FAIL test row
or a failed setup/fetch step is a PASS; a result for another sha is accepted; the stage names a die or a driver; the record reaching the
report is unbounded or carries a UUID; a secret-looking environment variable reaches the workload processes.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import kfml_stage as ks
import payload_instruction as pi
import vast_report

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SHA = "a" * 40


def ml_row(name="nanogpt_compile", status="PASS", reason="", wall=12.5, **over):
    row = {"name": name, "status": status, "reason": reason, "wall_s": wall, "gpu_name": "Some GPU", "compute_cap": "8.9", "vram_gb": 12.0,
           "driver": "560.35.03", "driver_cuda": "12.6", "flavor": "cu126", "torch": "2.7.1+cu126", "cuda": "12.6",
           "metrics": {"loss": 1.5, "ok": True, "note": "text", "nested": {"a": 1}, "log": "/x/y.log"}}
    row.update(over)
    return row


class Summarize(unittest.TestCase):
    def doc(self, rows):
        return ks.summarize(rows, SHA, "mid", [{"step": "run", "rc": 0, "s": 1.0}], "ciabc")

    def test_pass_needs_one_pass_and_no_fail(self):
        self.assertEqual(self.doc([ml_row(), ml_row("b", "NOT_RUN", "cc too low")])["verdict"], "PASS")
        self.assertEqual(self.doc([ml_row(), ml_row("b", "FAIL", "boom")])["verdict"], "FAIL")
        self.assertEqual(self.doc([])["verdict"], "FAIL")

    def test_all_not_run_is_not_run_never_fail(self):
        self.assertEqual(self.doc([ml_row("a", "NOT_RUN", "r"), ml_row("b", "NOT_RUN", "r")])["verdict"], "NOT_RUN")

    def test_facts_come_from_the_rows_and_metrics_are_scalars(self):
        d = self.doc([ml_row()])
        self.assertEqual(d["facts"]["gpu_name"], "Some GPU")
        self.assertEqual((d["facts"]["driver"], d["facts"]["driver_cuda"], d["facts"]["flavor"], d["facts"]["cuda"]), ("560.35.03", "12.6", "cu126", "12.6"))
        self.assertEqual(d["rows"][0]["metrics"], {"loss": 1.5, "ok": True})

    def test_uuids_are_scrubbed(self):
        t = "GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001 and fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001"
        self.assertNotRegex(ks.scrub(t), r"[0-9a-f]{8}-[0-9a-f]{4}")
        d = self.doc([ml_row(reason="x " + t)])
        self.assertNotRegex(json.dumps(d), r"[0-9a-f]{8}-[0-9a-f]{4}-")

    def test_failure_and_not_run_reasons_are_scrubbed_too(self):
        u = "fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001"
        self.assertNotIn(u, json.dumps(ks.failure(SHA, "mid", [], "bad " + u)))
        self.assertNotIn(u, json.dumps(ks.not_run(SHA, "mid", [], "bad " + u, ["a"])))

    def test_record_is_bounded_and_validates(self):
        rows = [ml_row("t%d" % i, reason="r" * 900, metrics={("m%d" % j): j for j in range(50)}) for i in range(20)]
        d = self.doc(rows)
        self.assertLessEqual(len(d["rows"]), 8)
        self.assertLessEqual(len(json.dumps(d)), 8192)
        vast_report.validate_kfml(d)

    def test_no_die_or_driver_is_named_in_the_stage(self):
        text = (HERE / "kfml_stage.py").read_text()
        self.assertIsNone(re.search(r"RTX|GeForce|Tesla|Blackwell|Ada\b|\b[0-9]{3}\.[0-9]{2}\b|sm_[0-9]|cu1[0-9]{2}", text))


class FakeRunner:
    """subprocess.run stand-in: serves setup/fetch/run by the `kfml` subcommand and writes what the real command would."""

    def __init__(self, setup_rc=0, fetch_rc=0, run_rc=0, rows=None, plan_rc=0, plan_line=None, venv_rc=0):
        self.calls, self.setup_rc, self.fetch_rc, self.run_rc = [], setup_rc, fetch_rc, run_rc
        self.rows = [ml_row()] if rows is None else rows
        self.plan_rc, self.plan_line, self.venv_rc = plan_rc, plan_line, venv_rc

    def __call__(self, cmd, **kw):
        self.calls.append((cmd, kw))

        class P:
            returncode, stdout, stderr = 0, "", ""
        p = P()
        sub = cmd[cmd.index("kfml") + 1] if "kfml" in cmd else ""
        if cmd[1:4] == ["-m", "venv", cmd[3]]:
            p.returncode = self.venv_rc
        elif sub == "setup" and "--plan" in cmd:
            p.returncode = self.plan_rc
            if self.plan_line and "stdout" in kw:
                kw["stdout"].write((self.plan_line + "\n").encode())
        elif sub == "setup":
            p.returncode = self.setup_rc
        elif sub == "fetch-data":
            p.returncode = self.fetch_rc
        elif sub == "run":
            p.returncode = self.run_rc
            Path(cmd[cmd.index("--json") + 1]).write_text(json.dumps(self.rows))
        elif sub == "list":
            p.stdout = json.dumps({"tests": [{"name": "nanogpt_compile"}, {"name": "stress_alloc"}]})
        return p


class Args:
    def __init__(self, d, **over):
        self.sha, self.checkout, self.prefix, self.out = SHA, str(ROOT), str(Path(d) / "p"), str(Path(d) / "o.json")
        self.tier, self.reference, self.record_reference, self.run_id = "mid", None, None, "ciabc"
        self.__dict__.update(over)


class Run(unittest.TestCase):
    def go(self, **kw):
        with tempfile.TemporaryDirectory() as d:
            fr = FakeRunner(**kw)
            return ks.run(Args(d), runner=fr), fr

    @staticmethod
    def subs(fr):
        return [c[0][c[0].index("kfml") + 1] + (" --plan" if "--plan" in c[0] else "") for c in fr.calls if "kfml" in c[0]]

    def test_green_run_plans_then_sets_up_and_fetches_together_then_runs_with_auto_flavour(self):
        doc, fr = self.go()
        self.assertEqual(doc["verdict"], "PASS")
        subs = self.subs(fr)
        self.assertEqual(subs[0], "setup --plan")
        self.assertEqual(sorted(subs[1:3]), ["fetch-data", "setup"])  # concurrent: either order
        self.assertEqual(subs[3], "run")
        setup_cmd = next(c[0] for c in fr.calls if "setup" in c[0] and "--plan" not in c[0] and "kfml" in c[0])
        self.assertNotIn("--flavor", setup_cmd)  # the flavour is never passed in: kfml derives it from the box
        self.assertIn("--nvcc-if-needed", setup_cmd)
        self.assertIn("--tier", fr.calls[-1][0])
        self.assertEqual([s["step"] for s in doc["steps"]], ["plan", "setup", "fetch-data", "run"])

    def test_the_datasets_download_while_the_wheels_install_not_after_them(self):
        """Both children are started before either is waited for: the fetch child must be able to finish before the setup child does."""
        import threading
        gate = threading.Event()
        order = []

        class Slow(FakeRunner):
            def __call__(self, cmd, **kw):
                sub = cmd[cmd.index("kfml") + 1] if "kfml" in cmd else ""
                if sub == "setup" and "--plan" not in cmd:
                    if not gate.wait(10):
                        order.append("setup waited in vain")
                    order.append("setup done")
                elif sub == "fetch-data":
                    order.append("fetch done")
                    gate.set()
                return super().__call__(cmd, **kw)
        with tempfile.TemporaryDirectory() as d:
            doc = ks.run(Args(d), runner=Slow())
        self.assertEqual(order, ["fetch done", "setup done"])
        self.assertEqual(doc["verdict"], "PASS")

    def test_fetch_data_runs_against_the_venv_directory_without_the_venv_python(self):
        _, fr = self.go()
        f = next(c for c in fr.calls if "fetch-data" in c[0])
        self.assertEqual(f[1]["env"]["KF_ML_HOME"], str(Path(f[1]["env"]["KF_ML_HOME"])))
        self.assertTrue(f[1]["env"]["KF_ML_HOME"].endswith("/venv"))
        self.assertNotIn("/venv/bin/python", f[0][0])

    def test_no_flavour_fits_is_a_not_run_verdict_with_the_named_reason(self):
        doc, fr = self.go(plan_rc=3, plan_line=json.dumps({"status": "NOT_RUN", "reason": "no pinned flavour can run on this box: x"}))
        self.assertEqual(doc["verdict"], "NOT_RUN")
        self.assertIn("no pinned flavour can run", doc["reason"])
        self.assertTrue(all(r["status"] == "NOT_RUN" and r["reason"] for r in doc["rows"]))
        self.assertEqual(len(fr.calls), 2)  # the plan, then only the test list

    def test_a_box_no_flavour_fits_downloads_nothing_and_creates_no_venv(self):
        doc, fr = self.go(plan_rc=3, plan_line=json.dumps({"status": "NOT_RUN", "reason": "r"}))
        self.assertFalse([c for c in fr.calls if "fetch-data" in c[0] or c[0][1:3] == ["-m", "venv"]
                          or ("setup" in c[0] and "--plan" not in c[0])])
        vast_report.validate_kfml(doc)

    def test_a_failed_plan_other_than_no_flavour_is_a_fail(self):
        doc, fr = self.go(plan_rc=2)
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("flavour plan failed", doc["reason"])

    def test_a_failed_venv_creation_is_a_fail_before_any_download(self):
        doc, fr = self.go(venv_rc=1)
        self.assertEqual(doc["verdict"], "FAIL")
        self.assertIn("venv creation failed", doc["reason"])
        self.assertFalse([c for c in fr.calls if "fetch-data" in c[0]])

    def test_failed_setup_or_fetch_is_fail(self):
        for kw, word in (({"setup_rc": 1}, "setup failed"), ({"fetch_rc": 1}, "fetch-data failed")):
            doc, _ = self.go(**kw)
            self.assertEqual(doc["verdict"], "FAIL")
            self.assertIn(word, doc["reason"])
            vast_report.validate_kfml(doc)

    def test_a_fail_row_is_a_fail_verdict(self):
        doc, _ = self.go(rows=[ml_row(), ml_row("b", "FAIL", "check failed", )], run_rc=1)
        self.assertEqual(doc["verdict"], "FAIL")
        vast_report.validate_kfml(doc)

    def test_missing_or_empty_rows_is_fail(self):
        doc, _ = self.go(rows=[])
        self.assertEqual(doc["verdict"], "FAIL")

    def test_crash_exit_code_cannot_pass(self):
        doc, _ = self.go(run_rc=2)
        self.assertEqual(doc["verdict"], "FAIL")

    def children_env(self, names):
        with patch.dict(os.environ, dict({n: "x" for n in names}, KEEP_ME="1")):
            _, fr = self.go()
        return [kw["env"] for cmd, kw in fr.calls[:3]]

    def test_provider_and_cloud_secrets_never_reach_the_children(self):
        for env in self.children_env(["VAST_API_TOKEN", "AWS_SECRET_ACCESS_KEY", "AWS_ACCESS_KEY_ID"]):
            for n in ("VAST_API_TOKEN", "AWS_SECRET_ACCESS_KEY", "AWS_ACCESS_KEY_ID"):
                self.assertNotIn(n, env)
            self.assertEqual(env.get("KEEP_ME"), "1")

    def test_generic_token_secret_password_key_names_never_reach_the_children(self):
        for env in self.children_env(["GITHUB_TOKEN", "DB_PASSWORD", "MY_SECRET", "SOME_API_KEY"]):
            for n in ("GITHUB_TOKEN", "DB_PASSWORD", "MY_SECRET", "SOME_API_KEY"):
                self.assertNotIn(n, env)

    def test_main_prints_one_result_line_and_exit_codes(self):
        import io
        import contextlib
        real_run = ks.run
        for kw, rc in (({}, 0), ({"plan_rc": 3}, 0), ({"fetch_rc": 1}, 1), ({"run_rc": 2}, 1), ({"run_rc": 1, "rows": [ml_row("b", "FAIL", "boom")]}, 1)):
            with tempfile.TemporaryDirectory() as d:
                out = io.StringIO()
                with patch.object(ks, "run", lambda a, kw=kw: real_run(a, runner=FakeRunner(**kw))), contextlib.redirect_stdout(out):
                    got = ks.main(["run", "--sha", SHA, "--checkout", str(ROOT), "--prefix", d + "/p", "--out", d + "/o.json"])
                self.assertEqual(got, rc)
                lines = [l for l in out.getvalue().splitlines() if l.startswith("KF_KFML_RESULT ")]
                self.assertEqual(len(lines), 1)
                self.assertEqual(json.loads(lines[0].split(" ", 1)[1]), json.loads(Path(d, "o.json").read_text()))

    def test_bad_sha_is_refused(self):
        self.assertEqual(ks.main(["run", "--sha", "zz", "--checkout", str(ROOT), "--prefix", "/nonexistent/x", "--out", "/nonexistent/o"]), 2)


class Payload(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("vast_payload_kfml", HERE / "vast_payload.py")
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def inst(self, **over):
        doc = dict(tier="large", windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
        doc.update(over)
        return pi.Instruction(**doc)

    def test_off_by_default_in_every_tier(self):
        m = self.m
        self.assertFalse(m.KFML_IN_MIDDLE_LARGE)
        for tier in ("small", "middle", "large", "adhoc"):
            self.assertNotIn("kfml", m.plan_stage_names(self.inst(tier=tier)))

    def test_selectable_by_only_in_middle_and_large_not_small_or_adhoc(self):
        m = self.m
        for tier in ("middle", "large"):
            self.assertEqual(m.plan_stage_names(self.inst(tier=tier), ("kfml",))[-1], "kfml")
        for tier in ("small", "adhoc"):
            self.assertNotIn("kfml", m.plan_stage_names(self.inst(tier=tier), ("kfml",)))
        with self.assertRaises(m.EvidenceError):
            m.select_stages(m.plan_stage_names(self.inst(tier="small"), ("kfml",)), ["kfml"], [])
        self.assertEqual(m.KFML_TIER, {"middle": "mid", "large": "large"})

    def test_runs_after_the_linux_guests_and_before_install_flow_and_windows(self):
        m = self.m
        names = m.plan_stage_names(self.inst(windows=True, linux_apps="middle"), ("kfml", "install_flow"))
        self.assertEqual(names[names.index("kfml") - 1], "apps")
        self.assertEqual(names[names.index("kfml") + 1], "install_flow")
        self.assertLess(names.index("kfml"), names.index("windows_gate"))

    def test_flag_true_puts_it_in_both_tiers_only(self):
        m = self.m
        with patch.object(m, "KFML_IN_MIDDLE_LARGE", True):
            for tier, want in (("small", False), ("middle", True), ("large", True), ("adhoc", False)):
                self.assertEqual("kfml" in m.plan_stage_names(self.inst(tier=tier)), want, tier)

    def test_dry_run_selects_it_and_touches_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            pi.write(self.inst(tier="middle"), str(path))
            env = {k: v for k, v in os.environ.items() if not k.startswith("KF_")}
            cmd = [sys.executable, str(HERE / "vast_payload.py"), "--dry-run", "--instruction", str(path)]
            ok = subprocess.run([*cmd, "--only", "kfml"], env=env, capture_output=True, text=True, cwd=d)
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("KF_PAYLOAD_PLAN stages=kfml planned=provision:cuda_host:merge_bar:cuda_guest:kfml", ok.stdout)
            plain = subprocess.run(cmd, env=env, capture_output=True, text=True, cwd=d)
            self.assertNotIn("kfml", plain.stdout)
            self.assertEqual(sorted(os.listdir(d)), ["i.json"])

    def line(self, **over):
        doc = ks.summarize([ml_row()], SHA, "mid", [{"step": "run", "rc": 0, "s": 1.0}], "ci1")
        doc.update(over)
        return "noise\nKF_KFML_RESULT " + json.dumps(doc) + "\nmore\n", doc

    def test_a_green_record_is_a_pass_row(self):
        m = self.m
        text, doc = self.line()
        result = {}
        self.assertEqual(m.kfml_rows(text, 0, SHA, result), [m.row("kfml", 1)])
        self.assertEqual(result["kfml"], doc)

    def not_run_text(self):
        nr = ks.not_run(SHA, "mid", [], "no pinned flavour can run on this box: x", ["a"])
        return "KF_KFML_RESULT " + json.dumps(nr) + "\n"

    def test_not_run_with_rc0_is_a_skipped_row_never_a_pass_never_a_fail(self):
        m = self.m
        self.assertEqual(m.kfml_rows(self.not_run_text(), 0, SHA, {}), [m.row("kfml", skipped=1)])

    def test_not_run_line_with_a_bad_exit_code_is_still_a_failing_stage(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.kfml_rows(self.not_run_text(), 1, SHA, {})

    def test_not_run_line_from_a_killed_stage_is_a_failure(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.kfml_rows(self.not_run_text(), 137, SHA, {})

    def test_stage_not_run_flows_through_to_a_skipped_row_and_a_valid_report(self):
        m = self.m
        with tempfile.TemporaryDirectory() as d:
            doc = ks.run(Args(d), runner=FakeRunner(plan_rc=3, plan_line=json.dumps({"status": "NOT_RUN", "reason": "no pinned flavour can run on this box: y"})))
            result = {}
            self.assertEqual(m.kfml_rows("KF_KFML_RESULT " + json.dumps(doc) + "\n", 0, SHA, result), [m.row("kfml", skipped=1)])
            Path(d, "r.json").write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}], "kfml": result["kfml"]}))
            vast_report.read_result_file(Path(d, "r.json"))

    def test_a_result_for_another_sha_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.kfml_rows(self.line()[0], 0, "c" * 40, {})

    def test_rows_refuse_everything_else(self):
        m = self.m
        red = ks.failure(SHA, "mid", [], "setup failed rc=1")
        for text, rc, sha in (
            (self.line(verdict="FAIL", reason="x")[0], 0, SHA), (self.line()[0], 1, SHA), (self.line()[0], 0, "b" * 40),
            ("no result line\n", 0, SHA), (self.line()[0] * 2, 0, SHA), ("KF_KFML_RESULT {not json}\n", 0, SHA),
            ("KF_KFML_RESULT " + json.dumps(red) + "\n", 0, SHA),
        ):
            with self.subTest(rc=rc, sha=sha[:3], t=text[:40]):
                with self.assertRaises(m.EvidenceError) as caught:
                    m.kfml_rows(text, rc, sha, {})
                self.assertEqual(caught.exception.name, "kfml")

    def test_red_record_is_kept_in_the_result(self):
        m = self.m
        red = ks.failure(SHA, "mid", [], "setup failed rc=1")
        result = {}
        with self.assertRaises(m.EvidenceError):
            m.kfml_rows("KF_KFML_RESULT " + json.dumps(red) + "\n", 1, SHA, result)
        self.assertEqual(result["kfml"]["reason"], "setup failed rc=1")

    def test_manual_only_table_names_it(self):
        self.assertEqual(self.m.MANUAL_ONLY_STAGES["kfml"], "KFML_IN_MIDDLE_LARGE")


class Report(unittest.TestCase):
    def good(self):
        return ks.summarize([ml_row(), ml_row("b", "NOT_RUN", "vram below the floor")], SHA, "large", [{"step": "run", "rc": 0, "s": 3.0}], "ci1")

    def write(self, d, doc, verdict="PASS"):
        p = Path(d) / "r.json"
        p.write_text(json.dumps({"verdict": verdict, "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}], "kfml": doc}))
        return p

    def test_reads_a_good_record(self):
        with tempfile.TemporaryDirectory() as d:
            vast_report.read_result_file(self.write(d, self.good()))

    def test_refuses_a_malformed_record(self):
        bad = []
        g = self.good()
        for mutate in (
            lambda x: x.update(verdict="MAYBE"), lambda x: x.update(tier="huge"), lambda x: x.update(sha="x"),
            lambda x: x.update(extra=1), lambda x: x["rows"][0].update(wall_s=-1), lambda x: x["rows"][0].update(status="OK"),
            lambda x: x["rows"][0]["metrics"].update(k="str"), lambda x: x["rows"].extend([x["rows"][0]] * 9),
            lambda x: x["facts"].update(secret="x"), lambda x: x["steps"].append({"step": "rm", "rc": 0, "s": 1}),
            lambda x: x["rows"][1].update(reason=""), lambda x: x["rows"][1].update(reason="café"),
            lambda x: x.update(verdict="NOT_RUN"),  # a PASS row inside a NOT_RUN verdict
            lambda x: x["rows"].append(dict(x["rows"][0], status="FAIL", reason="boom")),  # PASS verdict, a PASS row and a FAIL row
        ):
            import copy
            doc = copy.deepcopy(g)
            mutate(doc)
            bad.append(doc)
        for doc in bad:
            with tempfile.TemporaryDirectory() as d:
                with self.assertRaises(Exception):
                    vast_report.read_result_file(self.write(d, doc))

    def test_verdict_and_rows_must_agree(self):
        g = self.good()
        import copy
        for verdict, statuses, ok in (
            ("PASS", ["PASS", "NOT_RUN"], True), ("PASS", ["PASS", "FAIL"], False), ("PASS", ["NOT_RUN"], False),
            ("NOT_RUN", ["NOT_RUN", "NOT_RUN"], True), ("NOT_RUN", ["PASS", "NOT_RUN"], False),
        ):
            d = copy.deepcopy(g)
            d["verdict"] = verdict
            d["rows"] = [dict(copy.deepcopy(g["rows"][0]), status=x, reason="why") for x in statuses]
            if ok:
                vast_report.validate_kfml(d)
            else:
                with self.assertRaises(ValueError):
                    vast_report.validate_kfml(d)

    def test_row_and_step_counts_are_bounded(self):
        import copy
        d = copy.deepcopy(self.good())
        d["rows"] = [d["rows"][0]] * 9
        with self.assertRaises(ValueError):
            vast_report.validate_kfml(d)
        d = copy.deepcopy(self.good())
        d["steps"] = [d["steps"][0]] * 9
        with self.assertRaises(ValueError):
            vast_report.validate_kfml(d)

    def test_a_passing_report_cannot_carry_a_kfml_with_a_failing_row(self):
        doc = ks.summarize([ml_row(), ml_row("b", "FAIL", "boom")], SHA, "mid", [{"step": "run", "rc": 1, "s": 1.0}])
        self.assertEqual(doc["verdict"], "FAIL")
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(Exception):
                vast_report.read_result_file(self.write(d, doc, "PASS"))
            vast_report.read_result_file(self.write(d, doc, "FAIL"))

    def test_a_passing_report_cannot_carry_a_failed_kfml(self):
        fail = ks.failure(SHA, "mid", [], "setup failed rc=1")
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(Exception):
                vast_report.read_result_file(self.write(d, fail, "PASS"))
            vast_report.read_result_file(self.write(d, fail, "FAIL"))


if __name__ == "__main__":
    unittest.main()
