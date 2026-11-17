# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""CTS result parsing, the native-vs-guest diff and the chunked crash-resuming runner (cts_results.py, cts_run.py, kfopen_common.py). GPU-free."""
import contextlib
import io
import os
import tempfile
import unittest

import helpers as H
import cts_results as R
import cts_run as U
import kfopen_common as K
from kfopen_common import OpenRowError, Result

FAKE = K.FAKE_UUID_PREFIX
UUID_REAL_LOOKING = "0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e8"


def qpa_for(results, open_case=None, terminate=None, details=None):
    """A qpa text for {case: status}; `open_case` begins and never ends; `terminate` = (case, status) uses #terminateTestCaseResult."""
    out = ["#beginSession"]
    for name, st in results.items():
        out += ["#beginTestCaseResult " + name, "", '<TestCaseResult CasePath="%s">' % name,
                ' <Result StatusCode="%s">%s detail %s</Result>' % (st, st, (details or {}).get(name, "")),
                "</TestCaseResult>", "", "#endTestCaseResult"]
    if terminate:
        out += ["#beginTestCaseResult " + terminate[0], "", "<TestCaseResult>", "", "#terminateTestCaseResult " + terminate[1]]
    if open_case:
        out += ["#beginTestCaseResult " + open_case, "", "<TestCaseResult>", " <Text>working"]
    return "\n".join(out) + "\n"


class FakeDeqp:
    """Plays the glcts / deqp-vk binary: per case `plan[case]` is a status, or "CRASH" (the process dies in the case) or "HANG" (it never returns)."""

    def __init__(self, plan, gpu="NVIDIA GeForce RTX 4090, 580.88", details=None):
        self.plan, self.gpu, self.calls, self.details = plan, gpu, [], details or {}

    def run(self, argv, cwd=None, timeout=None, env=None, tee=None):
        if argv[0] == "nvidia-smi":
            return Result(0, self.gpu + "\n")
        cl = [a.split("=", 1)[1] for a in argv if a.startswith("--deqp-caselist-file=")][0]
        log = [a.split("=", 1)[1] for a in argv if a.startswith("--deqp-log-filename=")][0]
        cases = [l.strip() for l in open(cl) if l.strip() and not l.startswith("#")]
        self.calls.append(cases)
        done, text, rc, timed = {}, None, 0, False
        for c in cases:
            st = self.plan.get(c, "Pass")
            if st == "CRASH":
                open(log, "w").write(qpa_for(done, open_case=c))
                return Result(3221225477, "crash", 2.0)
            if st == "HANG":
                open(log, "w").write(qpa_for(done, open_case=c))
                return Result(124, "", 1800.0, True)
            done[c] = st
        open(log, "w").write(qpa_for(done, details=self.details))
        return Result(0, "", 1.0)


class Qpa(unittest.TestCase):
    def test_fixture_cases_statuses_and_details(self):
        cases, inflight = R.parse_qpa(H.fixture("qpa_mixed.qpa"))
        got = {c.name: c.status for c in cases}
        self.assertEqual("Pass", got["KHR-GL46.compute_shader.simple-compute"])
        self.assertEqual("Fail", got["KHR-GL46.compute_shader.dispatch-indirect"])
        self.assertEqual("NotSupported", got["KHR-GL46.tessellation_shader.tessellation_control_to_tessellation_evaluation.data_pass_through"])
        self.assertEqual("QualityWarning", got["KHR-GL46.copy_image.functional"])
        self.assertEqual(5, len(cases))

    def test_terminate_line_gives_status(self):
        cases, _ = R.parse_qpa(H.fixture("qpa_mixed.qpa"))
        t = [c for c in cases if c.name.endswith("spirv_modules_test")][0]
        self.assertEqual("Crash", t.status)
        self.assertEqual("terminated", t.detail)

    def test_unfinished_case_reported_inflight(self):
        _, inflight = R.parse_qpa(H.fixture("qpa_mixed.qpa"))
        self.assertEqual("KHR-GL46.sparse_buffer_tests.BufferStorageTest", inflight)

    def test_terminate_status_is_taken_from_the_line(self):
        cases, inflight = R.parse_qpa("#beginTestCaseResult a.b.c\n<x>\n#terminateTestCaseResult Timeout\n")
        self.assertEqual([("a.b.c", "Timeout")], [(c.name, c.status) for c in cases])
        self.assertIsNone(inflight)

    def test_open_case_at_end_of_file_is_inflight_not_a_result(self):
        cases, inflight = R.parse_qpa("#beginTestCaseResult a.b.c\n<Result StatusCode=\"Pass\">x</Result>\n#endTestCaseResult\n#beginTestCaseResult a.b.d\n<x>\n")
        self.assertEqual(["a.b.c"], [c.name for c in cases])
        self.assertEqual("a.b.d", inflight)

    def test_a_case_cut_off_by_the_next_begin_is_a_crash(self):
        text = "#beginTestCaseResult a.b.c\n<x>\n#beginTestCaseResult a.b.d\n<Result StatusCode=\"Pass\">ok</Result>\n#endTestCaseResult\n"
        cases, inflight = R.parse_qpa(text)
        self.assertEqual([("a.b.c", "Crash"), ("a.b.d", "Pass")], [(c.name, c.status) for c in cases])
        self.assertIsNone(inflight)

    def test_a_case_without_a_result_element_is_an_internal_error(self):
        cases, _ = R.parse_qpa("#beginTestCaseResult a.b.c\n<x/>\n#endTestCaseResult\n")
        self.assertEqual("InternalError", cases[0].status)

    def test_console_parser_reads_the_executor_format(self):
        cases = R.parse_console(H.fixture("console_sample.txt"))
        self.assertEqual(["Pass", "NotSupported", "Fail"], [c.status for c in cases])
        self.assertTrue(cases[0].name.endswith("create_sampler"))

    def test_console_parser_ignores_totals_and_unknown_words(self):
        self.assertEqual([], R.parse_console("Test case 'a.b.c'..\n  Banana (x)\nTest run totals:\n  Passed:        1/3 (33.3%)\n"))


class ResultsFiles(unittest.TestCase):
    def test_roundtrip_is_sorted_and_header_kept(self):
        text = R.format_results({"row": "r", "role": "guest"}, {"b.b.b": ("Fail", "x"), "a.a.a": "Pass"})
        header, res = R.read_results(text)
        self.assertEqual({"row": "r", "role": "guest"}, header)
        self.assertEqual(["a.a.a", "b.b.b"], list(res))
        self.assertEqual("Fail", res["b.b.b"])

    def test_not_a_results_file_refused(self):
        with self.assertRaisesRegex(OpenRowError, "RESULTS_FORMAT"):
            R.read_results("a.b.c\tPass\n")

    def test_duplicate_case_in_results_refused(self):
        with self.assertRaisesRegex(OpenRowError, "RESULTS_DUPLICATE"):
            R.read_results(R.HEADER + "\n# row=r\na.b.c\tPass\na.b.c\tFail\n")

    def test_control_with_duplicate_refused(self):
        with tempfile.TemporaryDirectory() as t:
            p = os.path.join(t, "c.tsv")
            open(p, "w").write(R.HEADER + "\n# row=r\na.b.c\tPass\na.b.c\tPass\n")
            with self.assertRaises(OpenRowError):
                R.read_results(open(p).read())

    def test_bad_line_refused(self):
        with self.assertRaisesRegex(OpenRowError, "RESULTS_FORMAT"):
            R.read_results(R.HEADER + "\n# row=r\nno-tab-here\n")

    def test_details_scrub_uuids(self):
        det = R.format_details({"a.b.c": ("Fail", "device %s lost" % UUID_REAL_LOOKING), "a.b.d": ("Pass", "ok")})
        self.assertNotIn(UUID_REAL_LOOKING, det)
        self.assertIn(FAKE, det)
        self.assertNotIn("a.b.d", det)

    def test_details_have_no_real_uuid(self):
        cases, _ = R.parse_qpa(H.fixture("qpa_mixed.qpa"))
        det = R.format_details({c.name: (c.status, c.detail) for c in cases})
        self.assertNotRegex(det, r"(?i)GPU-[0-9a-f]{8}-")
        self.assertIn(FAKE, det)

    def test_scrub_replaces_uuid_and_keeps_equalities(self):
        other = "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d"
        out = K.scrub_uuids("GPU-%s %s %s" % (UUID_REAL_LOOKING, UUID_REAL_LOOKING.upper(), other))
        a, b, c = out.split(" ")
        self.assertEqual(a, b)
        self.assertNotEqual(a, c)
        self.assertTrue(a.startswith(FAKE))
        self.assertNotIn(UUID_REAL_LOOKING, out.lower())

    def test_summary_counts_by_status_and_group(self):
        total, groups = R.summarize({"A.b.c.x": "Pass", "A.b.c.y": "Fail", "A.b.d.z": "Pass"})
        self.assertEqual({"Pass": 2, "Fail": 1}, total)
        self.assertEqual({"A.b.c": {"Pass": 1, "Fail": 1}, "A.b.d": {"Pass": 1}}, groups)


class Diff(unittest.TestCase):
    def test_classes(self):
        self.assertEqual("EQUAL", R.classify("Pass", "Pass"))
        self.assertEqual("GUEST_REGRESSION", R.classify("Pass", "Fail"))
        self.assertEqual("GUEST_REGRESSION", R.classify("QualityWarning", "Crash"))
        self.assertEqual("GUEST_LOSES_FEATURE", R.classify("Pass", "NotSupported"))
        self.assertEqual("GUEST_GAINS_FEATURE", R.classify("NotSupported", "Pass"))
        self.assertEqual("NATIVE_ONLY_FAIL", R.classify("Fail", "Pass"))
        self.assertEqual("DIFFERENT_FAILURE", R.classify("Fail", "Timeout"))
        self.assertEqual("QUALITY_DIFF", R.classify("Pass", "QualityWarning"))

    def test_classify_feature_loss(self):
        self.assertEqual("GUEST_LOSES_FEATURE", R.classify("CompatibilityWarning", "NotSupported"))

    def test_equal_lists_pass(self):
        d = R.diff_results({"a.b.c": "Pass", "a.b.d": "Fail"}, {"a.b.c": "Pass", "a.b.d": "Fail"})
        self.assertEqual("PASS", d["verdict"])
        self.assertEqual({"EQUAL": 2}, d["counts"])

    def test_difference_is_fail(self):
        d = R.diff_results({"a.b.c": "Pass", "a.b.d": "Pass"}, {"a.b.c": "Pass", "a.b.d": "Crash"})
        self.assertEqual("FAIL", d["verdict"])
        self.assertEqual([("a.b.d", "Pass", "Crash", "GUEST_REGRESSION")], d["rows"])

    def test_diff_candidates_include_feature_loss(self):
        d = R.diff_results({"a.b.c": "Pass", "a.b.d": "Pass", "a.b.e": "Fail"}, {"a.b.c": "NotSupported", "a.b.d": "Fail", "a.b.e": "Pass"})
        self.assertEqual({"a.b.c", "a.b.d"}, {r[0] for r in d["candidates"]})

    def test_missing_case_is_incomplete(self):
        d = R.diff_results({"a.b.c": "Pass", "a.b.d": "Pass"}, {"a.b.c": "Pass"})
        self.assertEqual("INCOMPLETE", d["verdict"])
        self.assertEqual(["a.b.d"], d["missing_in_guest"])
        d = R.diff_results({"a.b.c": "Pass"}, {"a.b.c": "Pass", "a.b.x": "Pass"})
        self.assertEqual(["a.b.x"], d["missing_in_native"])

    def test_missing_case_is_not_hidden_by_a_difference(self):
        d = R.diff_results({"a.b.c": "Pass", "a.b.d": "Pass"}, {"a.b.c": "Fail"})
        self.assertEqual("INCOMPLETE", d["verdict"])

    def test_diff_file_lists_every_difference(self):
        text = R.format_diff(R.diff_results({"a.b.c": "Pass", "a.b.d": "Pass"}, {"a.b.c": "Fail"}))
        self.assertIn("MISSING_IN_GUEST", text)
        self.assertIn("a.b.c\tnative=Pass\tguest=Fail\tGUEST_REGRESSION", text)


class Runner(unittest.TestCase):
    CASES = ["A.b.c.%02d" % i for i in range(10)]

    def run_cases(self, plan, chunk=4, **kw):
        fake = FakeDeqp(plan)
        with tempfile.TemporaryDirectory() as t:
            res, log = U.run_cases(fake, "glcts.exe", t, [], self.CASES, chunk, t, **kw)
        return res, log, fake

    def test_every_case_gets_one_status(self):
        res, log, _ = self.run_cases({self.CASES[2]: "Fail", self.CASES[5]: "CRASH", self.CASES[8]: "NotSupported"})
        self.assertEqual(set(self.CASES), set(res))
        self.assertEqual("Fail", res[self.CASES[2]][0])
        self.assertEqual("NotSupported", res[self.CASES[8]][0])

    def test_crash_marks_inflight_case(self):
        res, log, _ = self.run_cases({self.CASES[5]: "CRASH"})
        self.assertEqual("Crash", res[self.CASES[5]][0])
        self.assertTrue(any(a["died"] for a in log))

    def test_crash_resumes_remaining_cases(self):
        res, log, fake = self.run_cases({self.CASES[1]: "CRASH"})
        self.assertEqual("Pass", res[self.CASES[2]][0], "cases after the crash are run by a fresh process")
        self.assertEqual("Pass", res[self.CASES[9]][0])
        self.assertGreaterEqual(len(fake.calls), 3)

    def test_timeout_chunk_marks_timeout(self):
        res, _, _ = self.run_cases({self.CASES[3]: "HANG"})
        self.assertEqual("Timeout", res[self.CASES[3]][0])
        self.assertEqual("Pass", res[self.CASES[4]][0])

    def test_crash_handler_terminate_status_recorded(self):
        class Terminating(FakeDeqp):
            def run(self, argv, **kw):
                if argv[0] == "nvidia-smi":
                    return super().run(argv)
                log = [a.split("=", 1)[1] for a in argv if a.startswith("--deqp-log-filename=")][0]
                cl = [a.split("=", 1)[1] for a in argv if a.startswith("--deqp-caselist-file=")][0]
                cases = [l.strip() for l in open(cl) if l.strip() and not l.startswith("#")]
                open(log, "w").write(qpa_for({cases[0]: "Pass"}, terminate=(cases[1], "DeviceLost")))
                return Result(1, "", 1.0)
        with tempfile.TemporaryDirectory() as t:
            res, _ = U.run_cases(Terminating({}), "deqp-vk.exe", t, [], self.CASES[:2], 2, t)
        self.assertEqual("DeviceLost", res[self.CASES[1]][0])

    def test_crash_is_not_a_timeout(self):
        res, _, _ = self.run_cases({self.CASES[3]: "CRASH"})
        self.assertEqual("Crash", res[self.CASES[3]][0])

    def test_a_case_run_is_never_repeated_after_it_finished(self):
        _, _, fake = self.run_cases({self.CASES[1]: "CRASH"})
        flat = [c for call in fake.calls for c in call]
        self.assertEqual(1, flat.count(self.CASES[0]))

    def test_lost_qpa_falls_back_on_console_output(self):
        class Lost(FakeDeqp):
            def run(self, argv, cwd=None, timeout=None, env=None, tee=None):
                if argv[0] == "nvidia-smi":
                    return super().run(argv)
                out = "".join("\nTest case '%s'..\n  Pass (Pass)\n" % c for c in
                              [l.strip() for l in open([a.split("=", 1)[1] for a in argv if a.startswith("--deqp-caselist-file=")][0]) if l.strip() and not l.startswith("#")])
                return Result(0, out, 1.0)
        with tempfile.TemporaryDirectory() as t:
            res, _ = U.run_cases(Lost({}), "glcts.exe", t, [], self.CASES, 5, t)
        self.assertEqual({"Pass"}, {v[0] for v in res.values()})

    def test_a_runner_that_never_records_anything_gives_up_with_internal_error(self):
        class Dead:
            def run(self, argv, **kw):
                return Result(1, "")
        with tempfile.TemporaryDirectory() as t:
            res, log = U.run_cases(Dead(), "glcts.exe", t, [], self.CASES[:3], 3, t, max_attempts=8)
        self.assertEqual(set(self.CASES[:3]), set(res))
        self.assertEqual("Crash", res[self.CASES[0]][0])

    def test_chunk_files_carry_the_cases_and_the_row_args(self):
        fake = FakeDeqp({})
        with tempfile.TemporaryDirectory() as t:
            seen = []
            orig = fake.run
            fake.run = lambda argv, **kw: (seen.append(argv), orig(argv, **kw))[1]
            U.run_cases(fake, "glcts.exe", t, ["--deqp-gl-context-type=wgl"], self.CASES[:2], 2, t)
        self.assertIn("--deqp-gl-context-type=wgl", seen[0])
        self.assertTrue(seen[0][0].endswith("glcts.exe"))

    def test_rerun_adopts_only_equal(self):
        control = {c: "Pass" for c in self.CASES}
        fake = FakeDeqp({self.CASES[2]: "Fail"})
        with tempfile.TemporaryDirectory() as t:
            res, _ = U.run_cases(fake, "glcts.exe", t, [], self.CASES, 5, t)
            fake.plan = {}                                      # the rerun passes: a flake
            out, flaky = U.rerun_differences(fake, "glcts.exe", t, [], res, control, t)
        self.assertEqual([self.CASES[2]], flaky)
        self.assertEqual("Pass", out[self.CASES[2]][0])

    def test_persistent_difference_not_flaky(self):
        control = {c: "Pass" for c in self.CASES}
        fake = FakeDeqp({self.CASES[2]: "Fail"})
        with tempfile.TemporaryDirectory() as t:
            res, _ = U.run_cases(fake, "glcts.exe", t, [], self.CASES, 5, t)
            out, flaky = U.rerun_differences(fake, "glcts.exe", t, [], res, control, t)
        self.assertEqual([], flaky)
        self.assertEqual("Fail", out[self.CASES[2]][0])


class Verdict(unittest.TestCase):
    ROW = {"binary": "glcts", "chunk": 4, "args": ["--deqp-gl-context-type=wgl"]}

    def execute(self, plan, control_statuses=None, gpu="NVIDIA GeForce RTX 4090, 580.88", control_header=None, runner=None, **kw):
        fake = runner or FakeDeqp(plan, gpu)
        t = tempfile.mkdtemp(prefix="kfopen-run-")
        self.addCleanup(__import__("shutil").rmtree, t, True)
        cl = os.path.join(t, "cl.txt")
        open(cl, "w", newline="\n").write("# kfopen caselist test\n" + "\n".join(Runner.CASES) + "\n")
        control_path = None
        if control_statuses is not None:
            sha = K.sha256_text(open(cl, newline="").read())
            h = {"row": "vkglcts_gl46", "role": "native", "cts_commit": "c" * 40, "caselist_sha256": sha, "binary": "glcts", "gpu_name": "NVIDIA_GeForce_RTX_4090"}
            h.update(control_header or {})
            control_path = os.path.join(t, "control.tsv")
            open(control_path, "w").write(R.format_results(h, control_statuses))
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = U.execute_row("vkglcts_gl46", self.ROW, "c" * 40, cl, os.path.join(t, "glcts.exe"), t, fake, os.path.join(t, "out"), control_path=control_path, **kw)
        return rc, buf.getvalue(), t

    def test_no_control_is_notrun(self):
        rc, out, _ = self.execute({})
        self.assertEqual(3, rc)
        self.assertIn("KFNOTRUN vkglcts_gl46 reason=no-native-control", out)

    def test_discovery_prints_results_and_notrun(self):
        rc, out, t = self.execute({Runner.CASES[1]: "Fail"})
        self.assertIn("KFOPEN RESULTS row=vkglcts_gl46 cases=10 Fail=1 Pass=9", out)
        self.assertTrue(os.path.exists(os.path.join(t, "out", "vkglcts_gl46.guest.tsv")))
        self.assertNotIn("VERDICT PASS", out)

    def test_timeout_status_reaches_the_results_file(self):
        rc, out, t = self.execute({Runner.CASES[3]: "HANG"}, {c: ("Timeout" if c == Runner.CASES[3] else "Pass") for c in Runner.CASES})
        self.assertEqual(0, rc, out)
        _, res = R.read_results(open(os.path.join(t, "out", "vkglcts_gl46.guest.tsv")).read())
        self.assertEqual("Timeout", res[Runner.CASES[3]])

    def test_details_file_written_by_the_row_has_no_uuid(self):
        fake = FakeDeqp({Runner.CASES[2]: "Fail"}, details={Runner.CASES[2]: "device GPU-%s lost" % UUID_REAL_LOOKING})
        _, _, t = self.execute({}, runner=fake)
        text = open(os.path.join(t, "out", "vkglcts_gl46.guest.details.tsv")).read()
        self.assertIn(Runner.CASES[2], text)
        self.assertNotIn(UUID_REAL_LOOKING, text.lower())
        self.assertIn(FAKE, text)

    def test_equal_to_control_is_pass(self):
        rc, out, _ = self.execute({Runner.CASES[1]: "Fail"}, {c: ("Fail" if c == Runner.CASES[1] else "Pass") for c in Runner.CASES})
        self.assertEqual(0, rc)
        self.assertIn("KFOPEN VERDICT PASS row=vkglcts_gl46 cases=10 equal-to-control", out)

    def test_candidates_printed_and_row_fails(self):
        rc, out, _ = self.execute({Runner.CASES[4]: "CRASH"}, {c: "Pass" for c in Runner.CASES})
        self.assertEqual(1, rc)
        self.assertIn("KFOPEN CANDIDATE %s native=Pass guest=Crash GUEST_REGRESSION" % Runner.CASES[4], out)
        self.assertIn("CHECK cts_equals_control FAIL", out)

    def test_incomplete_is_notrun_not_pass(self):
        rc, out, _ = self.execute({}, {c: "Pass" for c in Runner.CASES[:8]})
        self.assertEqual(3, rc)
        self.assertIn("KFNOTRUN vkglcts_gl46 reason=incomplete-case-sets", out)

    def test_control_from_other_commit_refused(self):
        with self.assertRaisesRegex(OpenRowError, "CONTROL_MISMATCH cts_commit"):
            self.execute({}, {c: "Pass" for c in Runner.CASES}, control_header={"cts_commit": "d" * 40})

    def test_control_from_other_caselist_refused(self):
        with self.assertRaisesRegex(OpenRowError, "CONTROL_MISMATCH caselist_sha256"):
            self.execute({}, {c: "Pass" for c in Runner.CASES}, control_header={"caselist_sha256": "0" * 64})

    def test_execute_row_refuses_control_from_other_gpu(self):
        with self.assertRaisesRegex(OpenRowError, "CONTROL_MISMATCH gpu_name"):
            self.execute({}, {c: "Pass" for c in Runner.CASES}, control_header={"gpu_name": "NVIDIA_GeForce_RTX_3070"})

    def test_control_from_other_gpu_refused(self):
        h = {"cts_commit": "c", "caselist_sha256": "s", "binary": "glcts"}
        with self.assertRaises(OpenRowError):
            U.check_control(h, dict(h, gpu_name="RTX_3070"), "RTX_4090")
        self.assertTrue(U.check_control(h, dict(h, gpu_name="RTX_3070"), "RTX_4090", allow_different_gpu=True))

    def test_the_results_header_has_the_gpu_name_but_no_uuid(self):
        _, _, t = self.execute({})
        text = open(os.path.join(t, "out", "vkglcts_gl46.guest.tsv")).read()
        header, _ = R.read_results(text)
        self.assertEqual("NVIDIA_GeForce_RTX_4090", header["gpu_name"])
        self.assertNotRegex(text, r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-")

    def test_flaky_difference_is_resolved_by_one_rerun(self):
        class OnceFail(FakeDeqp):
            def __init__(self, case):
                super().__init__({})
                self.case, self.seen = case, False

            def run(self, argv, **kw):
                if argv[0] != "nvidia-smi":
                    cl = [a.split("=", 1)[1] for a in argv if a.startswith("--deqp-caselist-file=")][0]
                    if self.case in open(cl).read() and not self.seen:
                        self.plan, self.seen = {self.case: "Fail"}, True
                    else:
                        self.plan = {}
                return super().run(argv, **kw)
        rc, out, t = self.execute({}, {c: "Pass" for c in Runner.CASES}, runner=OnceFail(Runner.CASES[6]))
        self.assertEqual(0, rc)
        self.assertIn("KFOPEN FLAKY_RESOLVED count=1 first=%s" % Runner.CASES[6], out)
        self.assertIn("KFOPEN VERDICT PASS", out)

    def test_a_persistent_failure_is_not_resolved_as_a_flake(self):
        rc, out, _ = self.execute({Runner.CASES[6]: "Fail"}, {c: "Pass" for c in Runner.CASES})
        self.assertEqual(1, rc)
        self.assertNotIn("FLAKY_RESOLVED", out)

if __name__ == "__main__":
    unittest.main()
