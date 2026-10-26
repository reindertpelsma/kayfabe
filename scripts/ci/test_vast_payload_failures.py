#!/usr/bin/env python3
"""GPU-free falsifiers: a failed stage names its failed tests, and the merge bar is light.

Measured 2026-10-26 (live gate run 40263112941): `first_fail stage=merge_bar line=[redacted] --- FAILED`.
libtest's terse mode prints `<test name> --- FAILED` and the Worker/runner scrubber blanks any run of
32 or more [A-Za-z0-9_-] characters, so the test name, the only useful part, was lost. The summary
now carries the failed test names (at most 20), the first panic message, and long snake_case names
folded so a not-yet-redeployed scrubber cannot blank them; strings that are not test-name shaped are
never folded (they stay for the scrubber).
"""

import importlib.util
import json
import re
import unittest
from pathlib import Path

import test_vast_payload_summary as summary
from test_vast_payload import MERGE, SHA

LONG = "registered_tokens_skip_the_exit_and_everything_else_traps_in_order"
VECTORS = json.loads(
    (
        Path(__file__).resolve().parents[2] / "ci/vast-broker/test/fixtures/scrub-vectors.json"
    ).read_text()
)

# Captured from rustc 1.98.1, `./t -q`, with a 60-character test name.
TERSE = (
    "\nrunning 2 tests\n. 1/2\n"
    + LONG
    + " --- FAILED\n\nfailures:\n\n---- "
    + LONG
    + " stdout ----\n\nthread '"
    + LONG
    + "' (382263) panicked at t.rs:2:77:\n"
    "assertion `left == right` failed: left right mismatch\n  left: 1\n right: 2\n"
    "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n\n"
    "failures:\n    " + LONG + "\n\n"
    "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n"
)
VERBOSE = (
    "running 3 tests\ntest kf_chan::tests::alpha ... ok\ntest kf_chan::tests::beta ... FAILED\n"
    "test kf_chan::tests::gamma ... FAILED\n\nfailures:\n\n---- kf_chan::tests::beta stdout ----\n"
    "thread 'kf_chan::tests::beta' panicked at src/lib.rs:9:5:\nexplicit panic\n\n"
    "failures:\n    kf_chan::tests::beta\n    kf_chan::tests::gamma\n\n"
    "test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out\n"
)


def old_scrubber(text):
    """The rule that blanked the name: the Worker's and the runner's generic 32-character scrub."""
    return re.sub(r"[A-Za-z0-9_-]{32,}", "[redacted]", text)


class Base(unittest.TestCase):
    stages_run = summary.SummaryTests.stages_run
    block = summary.SummaryTests.block

    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "payload_failures", Path(__file__).with_name("vast_payload.py")
        )
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def logs(self, *texts):
        import tempfile

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        paths = []
        for i, text in enumerate(texts):
            path = Path(directory.name) / ("log%d" % i)
            path.write_text(text)
            paths.append(path)
        return paths


class FailedTestNames(Base):
    def test_terse_failure_line_names_the_test(self):
        names, total = self.m.failed_tests(self.logs(TERSE))
        self.assertEqual(names, [LONG])
        self.assertEqual(total, 1)

    def test_verbose_failures_and_list_are_merged_without_duplicates(self):
        names, total = self.m.failed_tests(self.logs(VERBOSE))
        self.assertEqual(names, ["kf_chan::tests::beta", "kf_chan::tests::gamma"])
        self.assertEqual(total, 2)

    def test_passing_tests_are_never_listed(self):
        text = "running 1 test\ntest a::b ... ok\ntest result: ok. 1 passed; 0 failed\n"
        self.assertEqual(self.m.failed_tests(self.logs(text)), ([], 0))

    def test_the_stdout_banner_alone_is_not_a_failure_list(self):
        text = "failures:\n\n---- a::b stdout ----\nboom\n"
        self.assertEqual(self.m.failed_tests(self.logs(text)), ([], 0))

    def test_at_most_20_names_are_returned_and_the_total_is_kept(self):
        text = "".join("test t::case_%02d ... FAILED\n" % i for i in range(25))
        names, total = self.m.failed_tests(self.logs(text))
        self.assertEqual(len(names), 20)
        self.assertEqual(names[0], "t::case_00")
        self.assertEqual(total, 25)

    def test_names_from_several_logs_are_joined_in_order(self):
        names, total = self.m.failed_tests(
            self.logs("x::first --- FAILED\n", "nothing here\n", "x::second --- FAILED\n")
        )
        self.assertEqual(names, ["x::first", "x::second"])

    def test_a_missing_log_is_skipped(self):
        names, _ = self.m.failed_tests([Path("/nonexistent/log")] + self.logs(TERSE))
        self.assertEqual(names, [LONG])

    def test_a_prose_line_ending_in_failed_is_not_a_test(self):
        text = "PURGE FAILED\nthe build FAILED\nGATE 3 FAILED\n"
        self.assertEqual(self.m.failed_tests(self.logs(text)), ([], 0))

    def test_odd_characters_in_a_name_are_refused(self):
        text = "bad name with spaces --- FAILED\n$(id)::x --- FAILED\n"
        self.assertEqual(self.m.failed_tests(self.logs(text)), ([], 0))


class FirstPanic(Base):
    def test_panic_location_and_message_are_returned(self):
        line = self.m.first_panic(self.logs(TERSE))
        self.assertIn("panicked at t.rs:2:77", line)
        self.assertIn("assertion `left == right` failed: left right mismatch", line)

    def test_only_the_first_panic_is_returned(self):
        line = self.m.first_panic(self.logs(VERBOSE + TERSE))
        self.assertIn("src/lib.rs:9:5", line)
        self.assertIn("explicit panic", line)
        self.assertNotIn("t.rs:2:77", line)

    def test_no_panic_gives_an_empty_string(self):
        self.assertEqual(self.m.first_panic(self.logs("all good\n")), "")

    def test_the_panic_line_is_bounded(self):
        text = "thread 'x' panicked at a.rs:1:1:\n" + "m" * 5000 + "\n"
        line = self.m.first_panic(self.logs(text))
        self.assertLessEqual(len(line), self.m.LINE_LIMIT)

    def test_a_panic_without_a_message_line_still_reports(self):
        line = self.m.first_panic(self.logs("thread 'x' panicked at a.rs:1:1:\n"))
        self.assertIn("panicked at a.rs:1:1", line)


class FoldLongNames(Base):
    def test_a_long_snake_case_name_is_folded_into_short_pieces(self):
        folded = self.m.fold_identifiers(LONG + " --- FAILED")
        self.assertTrue(all(len(piece) <= 30 for piece in re.findall(r"[A-Za-z0-9_-]+", folded)))
        self.assertEqual(folded.replace("~", ""), LONG + " --- FAILED")

    def test_folded_text_survives_the_old_scrubber(self):
        folded = self.m.fold_identifiers(LONG + " --- FAILED")
        self.assertNotIn("[redacted]", old_scrubber(folded))
        self.assertEqual(old_scrubber(folded).replace("~", ""), LONG + " --- FAILED")

    def test_short_names_are_left_alone(self):
        self.assertEqual(
            self.m.fold_identifiers("kf_chan::tests::beta --- FAILED"),
            "kf_chan::tests::beta --- FAILED",
        )

    def test_every_keep_vector_is_folded_below_the_scrub_threshold(self):
        for name, line in VECTORS["keep"].items():
            with self.subTest(name):
                self.assertNotIn("[redacted]", old_scrubber(self.m.fold_identifiers(line)))

    def test_no_token_shaped_vector_is_ever_folded(self):
        for name, token in VECTORS["redact"].items():
            with self.subTest(name):
                self.assertEqual(self.m.fold_identifiers("x " + token + " y"), "x " + token + " y")
                self.assertIn("[redacted]", old_scrubber(self.m.fold_identifiers(token)))


class SummaryCarriesTheFailures(Base):
    def failing(self, text, name="merge_bar"):
        specs = [dict(name=name, rc=101, text=text, fail="kvm_tests")]
        return self.block(self.stages_run(specs)[1])

    def test_summary_lists_each_failed_test_of_the_failed_stage(self):
        block = self.failing(VERBOSE)
        self.assertIn("failed_test stage=merge_bar name=kf_chan::tests::beta", block)
        self.assertIn("failed_test stage=merge_bar name=kf_chan::tests::gamma", block)

    def test_summary_carries_the_first_panic_message(self):
        block = self.failing(VERBOSE)
        panic = [s for s in block if s.startswith("panic stage=merge_bar line=")]
        self.assertEqual(len(panic), 1, block)
        self.assertIn("src/lib.rs:9:5", panic[0])
        self.assertIn("explicit panic", panic[0])

    def test_the_observed_defect_a_long_name_survives_the_old_scrubber(self):
        block = self.failing(TERSE)
        scrubbed = old_scrubber("\n".join(block))
        self.assertNotRegex(scrubbed, r"first_fail stage=merge_bar line=\[redacted\] --- FAILED")
        folded = scrubbed.replace("~", "")
        self.assertIn("failed_test stage=merge_bar name=" + LONG, folded)
        self.assertIn("first_fail stage=merge_bar line=" + LONG + " --- FAILED", folded)

    def test_failed_test_lines_are_capped_at_20_with_a_remainder_line(self):
        text = "".join("test t::case_%02d ... FAILED\n" % i for i in range(27))
        block = self.failing(text)
        listed = [s for s in block if s.startswith("failed_test ")]
        self.assertEqual(len(listed), 20)
        self.assertIn("failed_tests_more stage=merge_bar count=7", block)

    def test_the_summary_stays_within_4_kib_with_20_long_names(self):
        text = "".join("t::" + LONG + "_%02d --- FAILED\n" % i for i in range(30))
        block = self.failing(text)
        self.assertLessEqual(len(("\n".join(block) + "\n").encode()), 4096)
        self.assertEqual(block[-1], summary.END)
        self.assertTrue(any(s.startswith("first_fail stage=merge_bar") for s in block))

    def test_a_token_in_the_panic_message_is_not_laundered_past_the_scrubber(self):
        secret = VECTORS["redact"]["hex40"]
        text = (
            "thread 'x' panicked at a.rs:1:1:\nsaw "
            + secret
            + " and "
            + VECTORS["redact"]["base64url-mixed"]
            + "\n"
        )
        block = "\n".join(self.failing(text))
        self.assertIn(secret, block)
        scrubbed = old_scrubber(block)
        self.assertNotIn(secret, scrubbed)
        self.assertNotIn(VECTORS["redact"]["base64url-mixed"], scrubbed)

    def test_a_passing_stage_prints_no_failure_rows(self):
        _, out, _ = self.stages_run([dict(name="a", text=TERSE)])
        block = self.block(out)
        self.assertEqual([s for s in block if s.startswith(("failed_test", "panic"))], [])

    def test_non_ascii_and_escape_codes_in_names_are_removed(self):
        block = self.failing("\x1b[31mt::a‮b --- FAILED\x1b[0m\n")
        text = "\n".join(block)
        self.assertTrue(all(" " <= c <= "~" or c == "\n" for c in text), text)
        self.assertNotIn("\x1b", text)

    def test_failure_rows_come_from_the_detail_logs_too(self):
        # merge_bar's cargo output lives in <tag>_tests.log, a `detail` log, not in the stage output.
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            detail = Path(d) / "tag_tests.log"
            detail.write_text(VERBOSE)
            specs = [dict(name="merge_bar", rc=101, text="START\n", fail="kvm_tests")]
            stages_dir = Path(d)
            rc, out, _ = self.stages_run_with_detail(specs, detail)
        self.assertIn("failed_test stage=merge_bar name=kf_chan::tests::beta", self.block(out))

    def stages_run_with_detail(self, specs, detail):
        import contextlib, io, tempfile

        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            spec = specs[0]
            out = root / "merge_bar.run"
            progress = root / "merge_bar.progress"

            def grade(rc):
                raise self.m.EvidenceError("kvm_tests")

            stage = dict(
                name="merge_bar",
                command=["true"],
                output=out,
                progress=[progress],
                detail=[detail],
                grade=grade,
            )

            def runner(command, soft, output, prog):
                Path(output).write_text(spec["text"])
                prog[0].write_text("")
                return spec["rc"]

            result = {"verdict": "FAIL", "suites": [], "hardware": {}}
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                rc = self.m.execute_stages([stage], 10**13, result, root / "r.json", runner=runner)
            return rc, buffer.getvalue(), result


class MergeRows(Base):
    NAMES = ["gates", "kf3_build", "raw_suite", "thin_suite", "birth_census"]
    KVM_OK = "KVM_TESTS_REQUESTED=1\nKVM_TESTS passed 12 failed 0 kvm_ran=4 kvm_skipped=0\n"

    def with_kvm(self, extra):
        head, tail = MERGE.split("GATES_RC=0", 1)
        return head + extra + "GATES_RC=0" + tail

    def test_the_default_bar_has_no_crate_test_row(self):
        rows = self.m.merge_rows(MERGE, 0, SHA)
        self.assertEqual([r["name"] for r in rows], self.NAMES)

    def test_the_default_bar_does_not_need_a_totals_line(self):
        self.assertNotIn("TESTS passed", MERGE)
        self.m.merge_rows(MERGE, 0, SHA)

    def test_a_requested_kvm_stage_adds_its_row_first(self):
        rows = self.m.merge_rows(self.with_kvm(self.KVM_OK), 0, SHA)
        self.assertEqual([r["name"] for r in rows], ["kvm_tests"] + self.NAMES)
        self.assertEqual(rows[0]["pass"], 12)

    def test_a_requested_kvm_stage_with_no_result_refuses(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(self.with_kvm("KVM_TESTS_REQUESTED=1\n"), 0, SHA)
        self.assertEqual(caught.exception.name, "kvm_tests")

    def test_a_kvm_failure_names_the_kvm_suite(self):
        text = self.with_kvm("KVM_TESTS_REQUESTED=1\nKVM_TESTS_RC=101\n")
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(text, 1, SHA)
        self.assertEqual(caught.exception.name, "kvm_tests")

    def test_kvm_failed_totals_refuse(self):
        text = self.with_kvm(
            "KVM_TESTS_REQUESTED=1\nKVM_TESTS passed 10 failed 2 kvm_ran=4 kvm_skipped=0\n"
        )
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.merge_rows(text, 0, SHA)
        self.assertEqual(caught.exception.name, "kvm_tests")

    def test_kvm_skipped_or_unreached_gates_refuse(self):
        for tail in ("kvm_ran=4 kvm_skipped=1", "kvm_ran=0 kvm_skipped=0"):
            with self.subTest(tail):
                text = self.with_kvm(
                    "KVM_TESTS_REQUESTED=1\nKVM_TESTS passed 12 failed 0 " + tail + "\n"
                )
                with self.assertRaises(self.m.EvidenceError):
                    self.m.merge_rows(text, 0, SHA)

    def test_a_kvm_pass_line_without_a_request_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(
                self.with_kvm("KVM_TESTS passed 12 failed 0 kvm_ran=4 kvm_skipped=0\n"), 0, SHA
            )

    def test_a_duplicate_kvm_result_refuses(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.merge_rows(
                self.with_kvm(self.KVM_OK + self.KVM_OK.splitlines()[1] + "\n"), 0, SHA
            )

    def test_kvm_red_lines_stop_the_stage_early(self):
        self.assertTrue(self.m.red_line("KVM_TESTS_RC=101"))
        self.assertTrue(self.m.red_line("KVM_TESTS passed 10 failed 2 kvm_ran=4 kvm_skipped=0"))
        self.assertFalse(self.m.red_line("KVM_TESTS passed 12 failed 0 kvm_ran=4 kvm_skipped=0"))


if __name__ == "__main__":
    unittest.main()
