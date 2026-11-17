#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the image B `gpu` step diagnostics (2026-11-17, /root/night/GPU_UUID_MISMATCH_20261116.md, patch proposal A to C).

The step used to throw 'Post-fix seed GPU UUID mismatch' for no output, error text, two GPUs and a wrong value alike, polled inside one 120 s ssh
call, deleted the kf3 QEMU log with the stage's cleanup and recorded 'post-fix seed UUID exact match' from a fixed string even when the repair was
stubbed. These tests pin: the case is named (classification), the device is waited for BEFORE the restart and re-polled AFTER it from a bounded Python
loop, the diagnostics hold counts and PnP problem codes but never a UUID value, the log tail is kept and reported, and the row's detail is the real
result. Mutants: propimage_gpu_diag_mutants.py (two named killers each)."""
import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import test_propimage_apps_b_stage as tb  # noqa: E402
import test_propimage_check as tpc  # noqa: E402

m = tpc.m
EXPECTED = "GPU-11111111-2222-5333-8444-555555555555"
OTHER = "GPU-99999999-8888-5777-8666-555555555555"
HOST = "GPU-aaaaaaaa-bbbb-5ccc-8ddd-eeeeeeeeeeee"
STARTED = {"class": "Display", "present": True, "status": "OK", "problem": 0, "service": "nvlddmkm"}
BASIC = {"class": "Display", "present": True, "status": "OK", "problem": 0, "service": "BasicDisplay"}
CODE43 = {"class": "Display", "present": True, "status": "Error", "problem": 43, "service": "nvlddmkm"}
INSTALLING = {"class": "Display", "present": True, "status": "Error", "problem": 28, "service": ""}
PHANTOM = {"class": "Display", "present": False, "status": "Unknown", "problem": 45, "service": "nvlddmkm"}


def query(smi=(), rc=0, devices=(STARTED,)):
    return json.dumps({"smi_rc": rc, "smi_out": list(smi), "devices": list(devices)})


class Fake:
    """A guest and its clock: `script` is a function (n-th query, ms) -> query JSON; every sleep advances the clock."""

    def __init__(self, answers, repair_rc=0, events="== nvlddmkm events\n12:00:01 id=43 level=2 driver stopped\n"):
        self.answers, self.repair_rc, self.events = answers, repair_rc, events
        self.calls, self.now, self.queries = [], 0.0, 0

    def sleep(self, s):
        self.now += s

    def clock(self):
        return self.now

    def ps(self, script, timeout=120):
        self.now += 2
        if script == m.RENDER_GPU_QUERY:
            n, self.queries = self.queries, self.queries + 1
            self.calls.append("query")
            a = self.answers(n) if callable(self.answers) else self.answers[min(n, len(self.answers) - 1)]
            if a == "timeout":
                raise subprocess.TimeoutExpired("ssh", timeout)
            if type(a) is tuple:
                return SimpleNamespace(returncode=a[0], stdout=a[1], stderr="")
            return SimpleNamespace(returncode=0, stdout=a, stderr="")
        if script == m.render_gpu_fix():
            self.calls.append("repair")
            return SimpleNamespace(returncode=self.repair_rc, stdout=json.dumps({"changed": True, "restarted": 1}), stderr="boom " + OTHER if self.repair_rc else "")
        if script == m.RENDER_GPU_REENABLE:
            self.calls.append("reenable")
            return SimpleNamespace(returncode=0, stdout=json.dumps({"reenabled": 1}), stderr="")
        if script == m.RENDER_GPU_EVIDENCE:
            self.calls.append("events")
            return SimpleNamespace(returncode=0, stdout=self.events + "setupapi: " + EXPECTED + " {4d36e968-e325-11ce-bfc1-08002be10318}\n", stderr="")
        raise AssertionError("unexpected script")

    def fix(self, **kw):
        return m.gpu_fix(self, EXPECTED, sleep=self.sleep, clock=self.clock, host_fn=lambda: [HOST], **kw)


class Classify(unittest.TestCase):
    def kind(self, smi, rc=0, **kw):
        return m.classify_gpu_query({"smi_rc": rc, "smi_out": smi, "devices": [STARTED]}, EXPECTED, **kw)

    def test_each_case_has_its_own_name(self):
        self.assertEqual(self.kind([EXPECTED])["kind"], "expected")
        self.assertEqual(self.kind([])["kind"], "no_uuid")
        self.assertEqual(self.kind(["NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver."], rc=9)["kind"], "error_text")
        self.assertEqual(self.kind(["No devices were found"], rc=0)["kind"], "error_text")
        self.assertEqual(self.kind([EXPECTED, OTHER])["kind"], "multi")
        self.assertEqual(self.kind([OTHER])["kind"], "wrong_value")

    def test_a_failing_return_code_is_never_an_answer_even_with_the_right_line(self):
        self.assertEqual(self.kind([EXPECTED], rc=3)["kind"], "error_text")
        self.assertEqual(self.kind([], rc=None)["kind"], "error_text")  # nvidia-smi missing in the guest

    def test_wrong_value_says_whether_it_is_the_hosts(self):
        self.assertIs(self.kind([OTHER], host_uuids=[OTHER])["observed_is_host"], True)
        self.assertIs(self.kind([OTHER], host_uuids=[HOST])["observed_is_host"], False)
        self.assertIsNone(self.kind([OTHER])["observed_is_host"])

    def test_the_result_holds_counts_and_codes_never_a_value(self):
        for smi in ([EXPECTED], [OTHER], [EXPECTED, OTHER], ["error " + OTHER]):
            text = json.dumps(self.kind(smi, host_uuids=[HOST]))
            for secret in (EXPECTED, OTHER, HOST):
                self.assertNotIn(secret, text)
        self.assertEqual(self.kind([EXPECTED, OTHER])["gpu_lines"], 2)
        self.assertEqual(self.kind([OTHER])["devices"][0]["problem"], 0)

    def test_started_means_one_present_nvidia_service_device_with_status_ok(self):
        self.assertTrue(m.gpu_device_started([STARTED, PHANTOM]))  # a phantom is not present and not counted
        self.assertFalse(m.gpu_device_started([BASIC]))  # the basic display driver is not the NVIDIA driver
        self.assertFalse(m.gpu_device_started([CODE43]))
        self.assertFalse(m.gpu_device_started([INSTALLING]))
        self.assertFalse(m.gpu_device_started([]))
        self.assertFalse(m.gpu_device_started([STARTED, STARTED]))


class Loop(unittest.TestCase):
    def test_waits_for_the_device_to_be_started_before_the_restart(self):
        fake = Fake([query(devices=[INSTALLING]), query(devices=[BASIC]), query(devices=[STARTED]), query([EXPECTED])])
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertTrue(ok, d)
        self.assertEqual(fake.calls, ["query", "query", "query", "repair", "query"])
        self.assertTrue(d["started_before_restart"])
        self.assertGreaterEqual(d["wait_s"], 10)

    def test_gives_up_waiting_when_the_device_never_starts_and_says_so(self):
        fake = Fake([query(devices=[INSTALLING])])
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertFalse(ok)
        self.assertFalse(d["started_before_restart"])
        self.assertEqual(fake.calls.count("repair"), 1)  # still restarted once: the wait is bounded, not a gate that hangs the step
        self.assertLessEqual(fake.calls.index("repair"), 2 + m.GPU_WAIT_STARTED_S // 5)
        self.assertGreaterEqual(d["wait_s"], m.GPU_WAIT_STARTED_S)
        self.assertLess(d["wait_s"], m.GPU_WAIT_STARTED_S + 30)

    def test_repolls_after_the_restart_until_the_guest_answers(self):
        fake = Fake(lambda n: query(devices=[STARTED]) if n < 1 else (query([EXPECTED]) if n >= 6 else query([])))
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertTrue(ok, d)
        self.assertEqual(d["kind"], "expected")
        self.assertEqual([p["kind"] for p in d["polls"]], ["no_uuid"] * 5 + ["expected"])
        self.assertNotIn("events", d)  # collected on failure only

    def test_a_driver_that_never_answers_is_reported_absent_inside_a_bounded_budget(self):
        fake = Fake([query(devices=[STARTED]), query([], devices=[STARTED])])
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertFalse(ok)
        self.assertEqual(d["kind"], "no_uuid")
        self.assertEqual(d["gpu_lines"], 0)
        self.assertGreaterEqual(d["poll_s"], m.GPU_POLL_S)
        self.assertLess(d["poll_s"], m.GPU_POLL_S + 40)
        self.assertLess(len(fake.calls), 80)
        self.assertLessEqual(len(d["polls"]), 14)  # the record stays small

    def test_code_43_gets_one_disable_enable_at_half_the_budget_with_the_state_before_it(self):
        fake = Fake([query(devices=[STARTED]), query([], devices=[CODE43])])
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertEqual(fake.calls.count("reenable"), 1)
        self.assertGreaterEqual(d["reenable_at_s"], m.GPU_POLL_S // 2)
        self.assertEqual([x["problem"] for x in d["pnp_before_reenable"]], [43])
        self.assertEqual(d["problem_codes"], [43])

    def test_a_device_that_comes_back_after_the_disable_enable_passes(self):
        fake = Fake(lambda n: query(devices=[STARTED]) if n < 1 else (query([EXPECTED]) if "reenable" in fake.calls else query([], devices=[CODE43])))
        ok, text = fake.fix()
        self.assertTrue(ok, text)
        self.assertEqual(json.loads(text)["kind"], "expected")

    def test_two_gpus_and_a_wrong_value_stop_at_once_and_are_named(self):
        for smi, kind in (([EXPECTED, OTHER], "multi"), ([OTHER], "wrong_value")):
            fake = Fake([query(devices=[STARTED]), query(smi)])
            ok, text = fake.fix()
            d = json.loads(text)
            self.assertEqual((ok, d["kind"], len(d["polls"])), (False, kind, 1))
            self.assertIn("events", d)

    def test_a_timeout_and_a_transport_error_are_their_own_cases_and_keep_polling(self):
        fake = Fake(lambda n: query(devices=[STARTED]) if n < 1 else ("timeout" if n % 2 else (255, "ssh: connect refused")))
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertFalse(ok)
        self.assertEqual(sorted({p["kind"] for p in d["polls"]}), ["timeout", "transport"])
        self.assertGreater(len(fake.calls), 8)

    def test_a_right_looking_line_with_a_failing_return_code_is_not_a_pass(self):
        fake = Fake(lambda n: query(devices=[STARTED]) if n < 1 else query([EXPECTED], rc=9))
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertFalse(ok)
        self.assertEqual(d["kind"], "error_text")

    def test_a_guest_that_only_times_out_is_reported_as_a_timeout_not_as_absent(self):
        fake = Fake(lambda n: query(devices=[STARTED]) if n < 1 else "timeout")
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertEqual((ok, d["kind"]), (False, "timeout"))
        self.assertEqual({p["kind"] for p in d["polls"]}, {"timeout"})
        self.assertIn("observed UUID timeout", m.gpu_step_detail(ok, text))

    def test_the_whole_step_is_bounded_when_nothing_ever_works(self):
        fake = Fake([query(devices=[INSTALLING])])
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertFalse(ok)
        self.assertLess(d["total_s"], m.GPU_WAIT_STARTED_S + m.GPU_POLL_S + 120)
        self.assertGreaterEqual(d["total_s"], m.GPU_WAIT_STARTED_S + m.GPU_POLL_S)

    def test_a_wrong_value_is_final_and_does_not_burn_the_poll_budget(self):
        fake = Fake([query(devices=[STARTED]), query([OTHER])])
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertEqual(d["kind"], "wrong_value")
        self.assertLess(d["poll_s"], 30)
        self.assertEqual(fake.calls.count("reenable"), 0)

    def test_a_failed_repair_is_not_followed_by_polling(self):
        fake = Fake([query(devices=[STARTED])], repair_rc=1)
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertEqual(d["polls"] if "polls" in d else [], [])
        self.assertEqual(fake.calls.count("reenable"), 0)
        self.assertEqual(fake.calls[-1], "events")

    def test_a_failed_repair_is_named_and_the_text_is_kept(self):
        fake = Fake([query(devices=[STARTED])], repair_rc=1)
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertEqual((ok, d["kind"]), (False, "repair_failed"))
        self.assertIn("boom", d["repair_text"])
        self.assertNotIn(OTHER, text)
        self.assertIn("events", d)

    def test_failure_collects_the_guest_events_and_no_uuid_survives(self):
        fake = Fake([query(devices=[STARTED]), query([], devices=[CODE43])])
        ok, text = fake.fix()
        d = json.loads(text)
        self.assertIn("nvlddmkm", d["events"])
        self.assertIn("id=43", d["events"])
        self.assertIn("{4d36e968-e325-11ce-bfc1-08002be10318}", d["events"])  # a class GUID is not an identity
        self.assertNotIn(EXPECTED, text)
        self.assertEqual(fake.calls.count("events"), 1)

    def test_the_expected_value_is_validated(self):
        with self.assertRaises(ValueError):
            m.gpu_fix(Fake([query()]), "not a uuid; rm -rf")


class Detail(unittest.TestCase):
    def failed(self, answers):
        return Fake(answers).fix()

    def test_the_detail_names_the_case_the_counts_and_the_problem_code(self):
        ok, text = self.failed([query(devices=[STARTED]), query([], devices=[CODE43])])
        detail = m.gpu_step_detail(ok, text)
        self.assertTrue(detail.startswith("observed UUID no_uuid:"), detail)
        self.assertIn("expected=1 observed=0", detail)
        self.assertIn("problem code 43", detail)
        ok, text = self.failed([query(devices=[STARTED]), query([OTHER])])
        detail = m.gpu_step_detail(ok, text)
        self.assertIn("observed UUID wrong_value", detail)
        self.assertIn("host=not-host", detail)
        for secret in (EXPECTED, OTHER, HOST):
            self.assertNotIn(secret, detail)

    def test_a_pass_detail_comes_from_the_result(self):
        ok, text = Fake([query(devices=[STARTED]), query([EXPECTED])]).fix()
        detail = m.gpu_step_detail(ok, text)
        self.assertTrue(ok)
        self.assertIn("exact match after", detail)
        self.assertIn("problem code 0", detail)

    def test_a_repair_failure_is_named_in_the_detail_and_the_diag_file_hides_the_value(self):
        ok, text = Fake([query(devices=[STARTED])], repair_rc=1).fix()
        self.assertTrue(m.gpu_step_detail(ok, text).startswith("observed UUID repair_failed:"))
        with tempfile.TemporaryDirectory() as td:
            m.write_gpu_evidence(td, text)
            diag = (Path(td) / "gpu_diag.json").read_text()
            self.assertIn("boom", diag)
            self.assertNotIn(OTHER, diag)

    def test_a_stub_or_the_vfio_control_reports_its_own_text_never_exact_match(self):
        self.assertEqual(m.gpu_step_detail(True, "INVESTIGATION: gpu_fix skipped"), "INVESTIGATION: gpu_fix skipped")
        vfio = "VFIO control: kf3 seed repair and UUID check not applicable (real GPU)"
        self.assertEqual(m.gpu_step_detail(True, vfio), vfio)
        self.assertNotIn("exact match", m.gpu_step_detail(True, "INVESTIGATION: gpu_fix skipped"))
        self.assertTrue(m.gpu_step_detail(False, ""))

    def test_the_evidence_files_hold_the_case_and_the_events_without_a_uuid(self):
        ok, text = self.failed([query(devices=[STARTED]), query([], devices=[CODE43])])
        with tempfile.TemporaryDirectory() as td:
            m.write_gpu_evidence(td, text)
            names = sorted(p.name for p in Path(td).iterdir())
            self.assertEqual(names, ["gpu_diag.json", "gpu_events.txt", "gpu_fix.txt"])
            diag = json.loads((Path(td) / "gpu_diag.json").read_text())
            self.assertEqual(diag["kind"], "no_uuid")
            self.assertNotIn("events", diag)
            self.assertIn("nvlddmkm", (Path(td) / "gpu_events.txt").read_text())
            for p in Path(td).iterdir():
                self.assertNotIn(EXPECTED, p.read_text())

    def test_a_stub_result_writes_only_gpu_fix_txt(self):
        with tempfile.TemporaryDirectory() as td:
            m.write_gpu_evidence(td, "INVESTIGATION: gpu_fix skipped")
            self.assertEqual([p.name for p in Path(td).iterdir()], ["gpu_fix.txt"])


class QemuTail(unittest.TestCase):
    def test_the_tail_is_bounded_and_redacted(self):
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "qemu.log"
            log.write_text("".join("kf3: line %d\n" % i for i in range(500)) + "kf3: guest GPU UUID %s (gpu-uuid=seed)\n" % EXPECTED + "bare 12345678-1234-1234-1234-123456789abc\n")
            tail = m.qemu_log_tail(log, limit=50)
            self.assertEqual(len(tail.splitlines()), 50)
            self.assertNotIn(EXPECTED, tail)
            self.assertNotIn("12345678-1234", tail)
            self.assertIn("guest GPU UUID GPU-<redacted>", tail)
            self.assertIn("kf3: line 499", tail)
            self.assertNotIn("kf3: line 100\n", tail)

    def test_a_missing_log_is_said_not_raised(self):
        self.assertIn("unavailable", m.qemu_log_tail("/nonexistent/qemu.log"))

    def test_save_keeps_the_tail_in_out_and_prints_the_last_lines(self):
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "qemu.log"
            log.write_text("".join("kf3: line %d GPU-abc-def\n" % i for i in range(100)))
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                m.save_qemu_tail(td, log)
            self.assertEqual(len((Path(td) / "qemu_kf3_tail.txt").read_text().splitlines()), 100)
            self.assertIn("kf3: line 99", buf.getvalue())
            self.assertNotIn("kf3: line 60 ", buf.getvalue())
            self.assertNotIn("GPU-abc", buf.getvalue())


class Wiring(unittest.TestCase):
    """The whole harness (the pipeline of test_propimage_check): the gpu row, the files, and the kf3 log tail of a failed step."""

    def run_pipeline(self, smi, **kw):
        seen = {}

        def inspect(out):
            seen["files"] = sorted(p.name for p in out.iterdir())
            seen["diag"] = json.loads((out / "gpu_diag.json").read_text()) if (out / "gpu_diag.json").exists() else None

        buf = io.StringIO()
        rc, rows, apps = tpc.T("test_swtpm_supported").pipeline(gpu_smi=smi, inspect=inspect, stdout=buf, **kw)
        seen["stdout"] = buf.getvalue()
        return rows, seen, apps

    def test_a_failed_gpu_step_names_the_case_and_keeps_the_diagnostics_and_the_log_tail(self):
        rows, seen, apps = self.run_pipeline(())
        gpu = next(r for r in rows if r["step"] == "gpu")
        self.assertEqual(gpu["verdict"], "FAIL")
        self.assertTrue(gpu["detail"].startswith("observed UUID no_uuid:"), gpu["detail"])
        self.assertNotIn("exact match", gpu["detail"])
        for name in ("gpu_diag.json", "gpu_events.txt", "gpu_fix.txt", "qemu_kf3_tail.txt"):
            self.assertIn(name, seen["files"])
        self.assertEqual(seen["diag"]["kind"], "no_uuid")
        self.assertEqual(apps, 0)

    def test_a_passing_gpu_step_records_its_own_result_and_leaves_no_failure_files(self):
        rows, seen, apps = self.run_pipeline(("GPU-test",))
        gpu = next(r for r in rows if r["step"] == "gpu")
        self.assertEqual(gpu["verdict"], "PASS")
        self.assertIn("exact match after", gpu["detail"])
        self.assertNotIn("qemu_kf3_tail.txt", seen["files"])
        self.assertNotIn("gpu_events.txt", seen["files"])
        self.assertIn("gpu_diag.json", seen["files"])
        self.assertEqual(seen["diag"]["kind"], "expected")
        self.assertNotIn("kf3 QEMU log tail", seen["stdout"])

    def test_a_failed_gpu_step_prints_the_last_lines_of_the_kf3_log(self):
        tail = "".join("kf3: tail line %d\n" % i for i in range(80))
        with unittest.mock.patch.object(m, "qemu_log_tail", return_value=tail):
            rows, seen, apps = self.run_pipeline(())
        self.assertIn("kf3 QEMU log tail", seen["stdout"])
        self.assertIn("kf3: tail line 79", seen["stdout"])
        self.assertNotIn("kf3: tail line 49\n", seen["stdout"])

    def test_a_stubbed_repair_is_recorded_as_what_it_is(self):
        stub = lambda g, uuid: (True, "INVESTIGATION: gpu_fix skipped")  # noqa: E731  (the box wrapper hold_wrap.py)
        rows, seen, apps = self.run_pipeline(("GPU-other",), patch_gpu_fix=stub)
        gpu = next(r for r in rows if r["step"] == "gpu")
        self.assertEqual((gpu["verdict"], gpu["detail"]), ("PASS", "INVESTIGATION: gpu_fix skipped"))


class StageGpu(tb.RunBase, unittest.TestCase):
    """The image B stage: the kf3 QEMU log survives the cleanup (redacted), and a failed gpu step is reported with its case and the tail."""

    def boot(self, env, scenario, ctx, argv, mark):
        rc = super().boot(env, scenario, ctx, argv, mark)
        arg = lambda k: argv[argv.index(k) + 1]  # noqa: E731
        out, work = Path(arg("--out")), Path(arg("--work"))
        (work / "run/qemu.log").write_text("".join("kf3: line %d\n" % i for i in range(60)) + "kf3: guest GPU UUID %s (gpu-uuid=seed)\n" % EXPECTED)
        if scenario.get("_gpu_pass"):
            rows = json.loads((out / "result.json").read_text())
            rows.insert(1, {"step": "gpu", "verdict": "PASS", "evidence": "NONE", "class": "harness", "detail": "post-fix seed UUID exact match after 40s"})
            (out / "result.json").write_text(json.dumps(rows))
            (out / "gpu_diag.json").write_text(json.dumps({"kind": "expected"}))
        if scenario.get("_gpu_fail"):
            rows = json.loads((out / "result.json").read_text())
            rows.insert(1, {"step": "gpu", "verdict": "FAIL", "evidence": "NONE", "class": "harness",
                            "detail": "observed UUID no_uuid: expected=1 observed=0 host=unknown, problem code 43"})
            (out / "result.json").write_text(json.dumps(rows))
            (out / "gpu_diag.json").write_text(json.dumps({"kind": "no_uuid"}))
            (out / "gpu_events.txt").write_text("== nvlddmkm events\n")
            (out / "gpu_fix.txt").write_text("{}")
        return rc

    def run_stage(self, **scenario):
        env = self.make(**scenario)
        ev = env.tmp / "evidence"
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rec = tb.sb.run(env.ctx, str(ev))
        return env, ev, rec, buf.getvalue()

    def test_the_kf3_log_is_kept_after_the_cleanup_and_redacted(self):
        env, ev, rec, text = self.run_stage()
        self.assertFalse((env.ctx.prefix / "work").exists())  # the cleanup ran
        kept = sorted((env.ctx.prefix / "logs").glob("boot*_qemu_kf3.txt"))
        self.assertTrue(kept, "no kept qemu log")
        body = kept[0].read_text()
        self.assertIn("kf3: line 59", body)
        self.assertNotIn(EXPECTED, body)
        self.assertIn("GPU-<redacted>", body)

    def test_a_failed_gpu_step_prints_the_case_and_the_tail(self):
        env, ev, rec, text = self.run_stage(_gpu_fail=True)
        self.assertIn("KF_PROPIMAGE_APPS_B_GPU_FAIL boot=boot1 detail=observed UUID no_uuid", text)
        self.assertIn("KF_PROPIMAGE_APPS_B_GPU_QEMU_TAIL boot=boot1 kf3: line 59", text)
        self.assertNotIn(EXPECTED, text)
        self.assertIn("GPU-<redacted>", text)

    def test_a_failed_gpu_step_copies_the_diagnostic_files_and_the_kept_log(self):
        env, ev, rec, text = self.run_stage(_gpu_fail=True)
        copied = {p.name for p in (ev / "boot1").iterdir()}
        self.assertEqual(copied, {"gpu_fix.txt", "gpu_diag.json", "gpu_events.txt", "qemu_kf3_tail.txt"})
        self.assertNotIn(EXPECTED, (ev / "boot1/qemu_kf3_tail.txt").read_text())

    def test_a_passing_gpu_step_prints_nothing(self):
        env, ev, rec, text = self.run_stage(_gpu_pass=True)
        self.assertNotIn("KF_PROPIMAGE_APPS_B_GPU", text)

    def test_a_passing_gpu_step_copies_no_failure_files(self):
        env, ev, rec, text = self.run_stage(_gpu_pass=True)
        self.assertFalse((ev / "boot1").exists())
        self.assertEqual((rec["verdict"], rec["reason"]), ("PASS", ""))

    def test_a_boot_without_a_gpu_row_prints_nothing(self):
        env, ev, rec, text = self.run_stage()
        self.assertNotIn("KF_PROPIMAGE_APPS_B_GPU", text)


if __name__ == "__main__":
    unittest.main()
