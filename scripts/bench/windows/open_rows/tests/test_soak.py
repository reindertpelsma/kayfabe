# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GL dual-context soak judge and runner (gl_soak.py). GPU-free; the event log is a fixture."""
import contextlib
import io
import json
import unittest

import helpers as H
import gl_soak as S
import kfopen_pins as P
from kfopen_common import OpenRowError, Result

SPEC = P.load_pins()["rows"]["vkglcts_gl_dual_soak"]
OK_OUT = ("KFGL renderer NVIDIA GeForce RTX 4090/PCIe/SSE2\nKFGL version 4.6.0 NVIDIA 580.88\nKFGL start seconds=600 share=0\n"
          "KFGL tick t=5 a=100 b=90 bad=0/0\nKFGL tick t=10 a=200 b=181 bad=0/0\nKFGL RESULT OK iters=2000/1900\n")
CRASH = H.fixture_json("events_crash.json")


def judge(rc=0, out=OK_OUT, secs=601.0, events=(), smoke=False):
    return S.judge_soak(SPEC, rc, out, secs, list(events), smoke)


def failed(checks):
    return [n for n, ok, _ in checks if not ok]


class Facts(unittest.TestCase):
    def test_kfgl_lines(self):
        f = S.kfgl_facts(OK_OUT)
        self.assertEqual("NVIDIA GeForce RTX 4090/PCIe/SSE2", f["renderer"])
        self.assertEqual(2, len(f["ticks"]))
        self.assertTrue(f["result"].startswith("OK"))

    def test_events_parse(self):
        self.assertEqual([], S.parse_events(""))
        self.assertEqual([], S.parse_events("null"))
        self.assertEqual(1, len(S.parse_events('{"Id":1000,"Provider":"x","Message":"m"}')))
        self.assertEqual(3, len(S.parse_events(json.dumps(CRASH))))

    def test_unreadable_events_refused(self):
        with self.assertRaisesRegex(OpenRowError, "EVENTS_UNREADABLE"):
            S.parse_events("Get-WinEvent : no such log")

    def test_events_classified(self):
        c = S.classify_events(CRASH)
        self.assertEqual(1, len(c["crash_in_driver"]))
        self.assertEqual(1, len(c["app_crash"]))
        self.assertEqual(1, len(c["tdr"]))


class Judge(unittest.TestCase):
    def test_clean_run_passes(self):
        checks, f, ev = judge()
        self.assertEqual([], failed(checks), checks)

    def test_short_run_fails(self):
        self.assertIn("soak_duration", failed(judge(secs=120.0)[0]))

    def test_smoke_duration_is_checked(self):
        self.assertEqual([], failed(judge(secs=61.0, smoke=True)[0]))
        self.assertIn("soak_duration", failed(judge(secs=30.0, smoke=True)[0]))

    def test_nvoglv64_event_fails(self):
        ev = [CRASH[0]]
        self.assertIn("no_nvoglv_crash", failed(judge(events=ev)[0]))

    def test_wer_event_names_module(self):
        ev = [{"Id": 1001, "Provider": "Windows Error Reporting", "Message": "Faulting module: nvoglv64.dll"}]
        self.assertIn("no_nvoglv_crash", failed(judge(events=ev)[0]))

    def test_tdr_event_fails(self):
        self.assertIn("no_tdr", failed(judge(events=[CRASH[2]])[0]))

    def test_display_4101_counts(self):
        self.assertIn("no_tdr", failed(judge(events=[{"Id": 4101, "Provider": "Display", "Message": "recovered"}])[0]))

    def test_app_crash_of_our_exe_fails(self):
        self.assertIn("no_app_crash", failed(judge(events=[CRASH[1]])[0]))

    def test_crash_of_kf_gldual_named_in_a_wer_event_fails(self):
        ev = [{"Id": 1001, "Provider": "Windows Error Reporting", "Message": "APPCRASH P1: kf_gldual.exe P4: unknown"}]
        self.assertIn("no_app_crash", failed(judge(events=ev)[0]))

    def test_fail_result_line_fails(self):
        out = OK_OUT.replace("RESULT OK iters=2000/1900", "RESULT FAIL bad=3/0 first=compute checksum at iteration 7")
        self.assertIn("soak_result_ok", failed(judge(rc=1, out=out)[0]))

    def test_fail_result_line_fails_even_with_exit_zero(self):
        out = OK_OUT.replace("RESULT OK iters=2000/1900", "RESULT FAIL bad=3/0 first=compute checksum at iteration 7")
        self.assertIn("soak_result_ok", failed(judge(rc=0, out=out)[0]))

    def test_missing_result_line_fails(self):
        out = "\n".join(l for l in OK_OUT.splitlines() if "RESULT" not in l)
        self.assertIn("soak_result_ok", failed(judge(rc=0xC0000005, out=out)[0]))

    def test_missing_result_line_fails_even_with_exit_zero(self):
        out = "\n".join(l for l in OK_OUT.splitlines() if "RESULT" not in l)
        self.assertIn("soak_result_ok", failed(judge(rc=0, out=out)[0]))

    def test_ok_result_with_nonzero_exit_fails(self):
        self.assertIn("soak_result_ok", failed(judge(rc=1)[0]))

    def test_non_nvidia_renderer_fails(self):
        out = OK_OUT.replace("NVIDIA GeForce RTX 4090/PCIe/SSE2", "GDI Generic")
        self.assertIn("gl_renderer_nvidia", failed(judge(out=out)[0]))

    def test_missing_renderer_fails(self):
        out = "\n".join(l for l in OK_OUT.splitlines() if "renderer" not in l)
        self.assertIn("gl_renderer_nvidia", failed(judge(out=out)[0]))


class Run(unittest.TestCase):
    def runner(self, out=OK_OUT, rc=0, secs=605.0, events="[]"):
        def run(argv, **kw):
            if argv[0] == "powershell.exe":
                return Result(0, events)
            return Result(rc, out, secs)
        return H.FakeRunner([(lambda a: True, lambda a, c: run(a))])

    def go(self, runner, **kw):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = S.run_soak("vkglcts_gl_dual_soak", SPEC, runner, "kf_gldual.exe", **kw)
        return rc, buf.getvalue()

    def test_clean_soak_is_pass_and_asks_for_600_seconds(self):
        r = self.runner()
        rc, out = self.go(r)
        self.assertEqual(0, rc)
        self.assertEqual(["kf_gldual.exe", "--seconds", "600"], r.argvs()[0])
        self.assertIn("KFOPEN VERDICT PASS row=vkglcts_gl_dual_soak seconds=600 share=0", out)

    def test_event_query_goes_through_the_thin_ps1_with_the_start_time(self):
        r = self.runner()
        self.go(r)
        ps = [a for a in r.argvs() if a[0] == "powershell.exe"][0]
        self.assertTrue(any(x.endswith("open_events.ps1") for x in ps))
        self.assertEqual("-SinceUtc", ps[-2])
        self.assertRegex(ps[-1], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")

    def test_crash_in_driver_is_a_fail_with_check_line(self):
        rc, out = self.go(self.runner(events=json.dumps([CRASH[0]]), rc=1, out=OK_OUT.replace("OK", "FAIL")))
        self.assertEqual(1, rc)
        self.assertIn("CHECK no_nvoglv_crash FAIL", out)
        self.assertIn("KFOPEN VERDICT FAIL row=vkglcts_gl_dual_soak", out)

    def test_smoke_and_share_flags_reach_the_binary(self):
        r = self.runner(secs=62.0)
        rc, out = self.go(r, smoke=True, share=True)
        self.assertEqual(["kf_gldual.exe", "--seconds", "60", "--share"], r.argvs()[0])
        self.assertEqual(0, rc)

    def test_unreadable_event_output_fails_the_run(self):
        def run(argv, **kw):
            return Result(0, "Get-WinEvent : no such log") if argv[0] == "powershell.exe" else Result(0, OK_OUT, 605.0)
        rc, out = self.go(H.FakeRunner([(lambda a: True, lambda a, c: run(a))]))
        self.assertEqual(1, rc)
        self.assertIn("EVENTS_UNREADABLE", out)

    def test_an_event_query_that_fails_is_a_failed_check_not_a_silent_pass(self):
        def run(argv, **kw):
            return Result(1, "") if argv[0] == "powershell.exe" else Result(0, OK_OUT, 605.0)
        rc, out = self.go(H.FakeRunner([(lambda a: True, lambda a, c: run(a))]))
        self.assertIn("CHECK event_query FAIL", out)
        self.assertEqual(1, rc)
        self.assertIn("KFOPEN VERDICT FAIL", out)


if __name__ == "__main__":
    unittest.main()
