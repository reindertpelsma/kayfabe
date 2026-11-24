#!/usr/bin/env python3
"""The copy budget and the copy progress of the SPECviewperf stage (scripts/bench/specviewperf/copy_budget.py), GPU-free.

STATUS: CODE, 2026-11-20; GPU-free tests only. Mutants: scripts/ci/spec_stage_mutants.py (every mutant of copy_budget.py
needs two killers, one of them in this file or in test_vast_spec_stage.py).

FALSIFIER: the budget of the copy into the guest is a number somebody typed (the old fixed 4200 s) and not the lane's size at
a stated floor rate; the knob KF_SPEC_COPY_BUDGET_S is ignored or a bad value of it is accepted; the progress filter changes a
byte of the stream; a copy that ran leaves no progress line; the abort line of a copy that ran out of time does not say how many
bytes arrived, at which rate, and what budget would have sufficed.
"""
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SPECD = ROOT / "scripts/bench/specviewperf"
HELPER = SPECD / "copy_budget.py"
sys.path.insert(0, str(SPECD))
import copy_budget as cb  # noqa: E402

GIB = 1 << 30
# the lane table's sizes (tree_gb, read as GiB) and the budget the rule gives them: floor 5 MB/s, +20 %, +600 s
LANE_A_BYTES = 53579717018
LANE_B_BYTES = 47137266073
BOTH_BYTES = 100716983091
LANE_A_BUDGET_S = 13460
LANE_B_BUDGET_S = 11913
BOTH_BUDGET_S = 24773


def run(*args, stdin=b"", timeout=60):
    return subprocess.run([sys.executable, "-I", str(HELPER), *args], input=stdin, capture_output=True, timeout=timeout)


class Knob(unittest.TestCase):
    def test_a_whole_number_of_seconds_from_600_to_86400_is_accepted(self):
        for text, want in (("600", 600), ("4200", 4200), ("86400", 86400), ("13460", 13460)):
            with self.subTest(text=text):
                self.assertEqual(cb.parse_knob(text), want)

    def test_an_unset_or_empty_knob_is_the_default_not_an_error(self):
        self.assertIsNone(cb.parse_knob(None))
        self.assertIsNone(cb.parse_knob(""))

    def test_a_leading_zero_is_decimal_never_octal(self):
        self.assertEqual(cb.parse_knob("0700"), 700)

    def test_a_bad_value_is_refused_by_name(self):
        for text in ("599", "0", "86401", "100000", "abc", "1e3", "-600", "+600", "600.5", " 600", "600 ", "0x300", "6 00", "٦٠٠", "600\n"):
            with self.subTest(text=text):
                with self.assertRaises(cb.KnobError) as cm:
                    cb.parse_knob(text)
                self.assertIn("KF_SPEC_COPY_BUDGET_S", str(cm.exception))
                self.assertIn("600", str(cm.exception))
                self.assertIn("86400", str(cm.exception))

    def test_the_progress_interval_knob_is_one_to_3600_seconds_and_defaults_to_60(self):
        self.assertEqual(cb.parse_note_s(None), 60)
        self.assertEqual(cb.parse_note_s(""), 60)
        self.assertEqual(cb.parse_note_s("1"), 1)
        self.assertEqual(cb.parse_note_s("3600"), 3600)
        for text in ("0", "3601", "x", "-1", "1.5"):
            with self.subTest(text=text):
                with self.assertRaises(cb.KnobError) as cm:
                    cb.parse_note_s(text)
                self.assertIn("KF_SPEC_COPY_NOTE_S", str(cm.exception))


class Rates(unittest.TestCase):
    def test_rate_in_decimal_megabytes_per_second(self):
        self.assertEqual(cb.rate_mb_s(5_000_000, 1), 5.0)
        self.assertEqual(cb.rate_mb_s(822_000_000, 60), 13.7)

    def test_nothing_copied_or_no_time_gone_is_a_rate_of_zero_never_a_division_error(self):
        self.assertEqual(cb.rate_mb_s(0, 60), 0.0)
        self.assertEqual(cb.rate_mb_s(10, 0), 0.0)
        self.assertEqual(cb.rate_mb_s(0, 0), 0.0)

    def test_the_budget_that_would_have_sufficed_is_the_whole_size_at_the_observed_rate(self):
        # 1 GB in 200 s is 5 MB/s: the 43.9 GB of lane B needs 8780 s at that rate
        self.assertEqual(cb.needed_s(43_900_000_000, 1_000_000_000, 200), 8780)

    def test_needed_is_rounded_up(self):
        self.assertEqual(cb.needed_s(10, 3, 1), 4)

    def test_nothing_copied_gives_no_estimate(self):
        self.assertIsNone(cb.needed_s(43_900_000_000, 0, 200))
        self.assertIsNone(cb.needed_s(43_900_000_000, 5, 0))

    def test_more_bytes_than_expected_never_estimates_less_time_than_already_spent(self):
        self.assertGreaterEqual(cb.needed_s(100, 500, 60), 60)


class Lines(unittest.TestCase):
    def test_the_progress_line_has_the_four_fields_in_order(self):
        self.assertEqual(
            cb.format_progress(1_500_000, 1.9, 4200),
            "SPEC_NOTE copy progress bytes=1500000 rate_mb_s=0.8 elapsed_s=1 budget_s=4200",
        )

    def test_a_progress_line_with_nothing_copied_says_zero(self):
        self.assertEqual(cb.format_progress(0, 60.2, 4200), "SPEC_NOTE copy progress bytes=0 rate_mb_s=0.0 elapsed_s=60 budget_s=4200")

    def test_the_last_progress_line_is_marked(self):
        line = cb.format_progress(10_000_000, 2, 600, final=True)
        self.assertEqual(line, "SPEC_NOTE copy progress bytes=10000000 rate_mb_s=5.0 elapsed_s=2 budget_s=600 final=1")
        self.assertNotIn("final", cb.format_progress(10_000_000, 2, 600))

    def test_the_abort_text_names_the_bytes_the_rate_and_the_budget_that_would_have_sufficed(self):
        text = cb.format_abort(1_000_000_000, 200, 1000, 43_900_000_000)
        self.assertIn("did not finish inside its budget", text)
        self.assertIn("bytes=1000000000", text)
        self.assertIn("expected_bytes=43900000000", text)
        self.assertIn("rate_mb_s=5.0", text)
        self.assertIn("elapsed_s=200", text)
        self.assertIn("budget_s=1000", text)
        self.assertIn("needed_s≈" + "8780", text)
        self.assertIn("KF_SPEC_COPY_BUDGET_S", text)
        self.assertNotIn("\n", text)

    def test_the_abort_text_of_a_copy_that_moved_nothing_says_so_and_gives_no_estimate(self):
        text = cb.format_abort(0, 300, 300, 43_900_000_000)
        self.assertIn("bytes=0", text)
        self.assertIn("rate_mb_s=0.0", text)
        self.assertIn("needed_s≈unknown", text)


class Derivation(unittest.TestCase):
    def test_the_floor_rate_is_a_named_constant_marked_measured_on_one_box_with_its_date(self):
        self.assertEqual(cb.FLOOR_RATE_BYTES_S, 5_000_000)
        src = HELPER.read_text()
        i = src.index("FLOOR_RATE_BYTES_S =")
        near = src[max(0, i - 900) : i + 400]
        self.assertIn("MEASURED 2026-11-20", near)
        self.assertIn("one box", near)

    def test_no_fixed_70_minute_budget_survives_in_the_module(self):
        self.assertNotIn("4200", HELPER.read_text())

    def test_the_size_of_a_lane_is_the_sum_of_its_viewsets_trees(self):
        self.assertEqual(cb.expected_bytes("lanea"), LANE_A_BYTES)
        self.assertEqual(cb.expected_bytes("laneb"), LANE_B_BYTES)
        self.assertEqual(cb.expected_bytes("both"), BOTH_BYTES)

    def test_the_size_is_the_lanes_viewsets_only_in_a_narrowed_table(self):
        tbl = json.loads((SPECD / "lanes.json").read_text())
        tbl["lanes"]["lanea"] = ["energy-04", "enscape-01"]
        self.assertEqual(cb.expected_bytes("lanea", tbl), round(1.6 * GIB) + round(2.2 * GIB))

    def test_an_unknown_lane_is_an_error(self):
        with self.assertRaises(ValueError):
            cb.expected_bytes("lanex")

    def test_the_default_is_the_size_at_the_floor_rate_plus_20_percent_plus_600_s(self):
        self.assertEqual(cb.derived_budget_s(LANE_A_BYTES), LANE_A_BUDGET_S)
        self.assertEqual(cb.derived_budget_s(LANE_B_BYTES), LANE_B_BUDGET_S)
        self.assertEqual(cb.derived_budget_s(BOTH_BYTES), BOTH_BUDGET_S)
        # 5 MB/s of 1 GB is 200 s, +20 % is 240 s, +600 s
        self.assertEqual(cb.derived_budget_s(1_000_000_000), 840)

    def test_a_copy_of_nothing_still_gets_the_fixed_allowance_which_is_the_knobs_lower_bound(self):
        self.assertEqual(cb.derived_budget_s(0), 600)
        self.assertEqual(cb.derived_budget_s(0), cb.KNOB_MIN_S)

    def test_the_default_of_every_lane_is_inside_the_knob_range(self):
        for lane in ("lanea", "laneb", "both"):
            with self.subTest(lane=lane):
                self.assertTrue(cb.KNOB_MIN_S <= cb.derived_budget_s(cb.expected_bytes(lane)) <= cb.KNOB_MAX_S)

    def test_the_old_70_minutes_is_far_below_what_the_measured_5_mb_s_needs_for_any_lane(self):
        # the incident of 2026-11-20: 5 MB/s, lane B, 70 minutes were not enough
        self.assertGreater(cb.derived_budget_s(LANE_B_BYTES), 70 * 60 * 2)


class Budget(unittest.TestCase):
    def test_without_a_knob_the_budget_is_derived_from_the_lane(self):
        self.assertEqual(cb.budget("lanea", None, 10**9), (LANE_A_BUDGET_S, "derived", LANE_A_BYTES, LANE_A_BUDGET_S))
        self.assertEqual(cb.budget("laneb", "", 10**9)[0], LANE_B_BUDGET_S)

    def test_the_knob_wins_over_the_derived_budget(self):
        self.assertEqual(cb.budget("lanea", "7200", 10**9), (7200, "knob", LANE_A_BYTES, 7200))
        self.assertEqual(cb.budget("laneb", "86400", 10**9)[0], 86400)

    def test_the_budget_never_exceeds_the_time_left(self):
        got = cb.budget("lanea", None, 5000)
        self.assertEqual(got, (5000, "derived", LANE_A_BYTES, LANE_A_BUDGET_S))
        got = cb.budget("lanea", "20000", 5000)
        self.assertEqual((got[0], got[1], got[3]), (5000, "knob", 20000))

    def test_a_budget_below_the_time_left_is_not_changed_by_it(self):
        self.assertEqual(cb.budget("lanea", "900", 5000)[0], 900)

    def test_no_time_left_is_a_budget_of_one_second_never_zero(self):
        # `timeout 0` would mean no timeout at all
        self.assertEqual(cb.budget("lanea", None, 0)[0], 1)
        self.assertEqual(cb.budget("lanea", "900", -50)[0], 1)

    def test_a_bad_knob_is_an_error_here_too(self):
        with self.assertRaises(cb.KnobError):
            cb.budget("lanea", "599", 10**9)

    def test_the_command_line_prints_budget_source_bytes_and_unclamped_budget(self):
        p = run("budget", "--lane", "laneb", "--knob", "", "--left", "9999")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout.decode().split()[:4], ["9999", "derived", str(LANE_B_BYTES), str(LANE_B_BUDGET_S)])
        self.assertIn("clamped to the time left (wanted %ds)" % LANE_B_BUDGET_S, p.stdout.decode())
        p = run("budget", "--lane", "laneb", "--knob", "1200", "--left", "99999")
        self.assertEqual(p.stdout.decode().split()[:4], ["1200", "knob", str(LANE_B_BYTES), "1200"])
        self.assertEqual(p.stdout.decode().count("\n"), 1)

    def test_the_command_line_refuses_a_bad_knob_by_name_with_status_2_and_prints_no_budget(self):
        p = run("budget", "--lane", "laneb", "--knob", "86401", "--left", "99999")
        self.assertEqual(p.returncode, 2)
        self.assertEqual(p.stdout, b"")
        self.assertIn(b"KF_SPEC_COPY_BUDGET_S", p.stderr)

    def test_the_command_line_refuses_an_unknown_lane(self):
        p = run("budget", "--lane", "lanex", "--knob", "", "--left", "99999")
        self.assertEqual(p.returncode, 2)
        self.assertEqual(p.stdout, b"")

    def test_the_note_says_where_the_number_comes_from(self):
        derived = cb.format_budget_note(LANE_A_BUDGET_S, "derived", LANE_A_BYTES, LANE_A_BUDGET_S)
        self.assertEqual(derived, "derived: 53579717018 bytes at the floor of 5 MB/s (MEASURED 2026-11-20 on one box), +20% and 600 s")
        self.assertEqual(cb.format_budget_note(7777, "knob", LANE_A_BYTES, 7777), "set by KF_SPEC_COPY_BUDGET_S")
        self.assertEqual(
            cb.format_budget_note(5000, "knob", LANE_A_BYTES, 20000),
            "set by KF_SPEC_COPY_BUDGET_S; clamped to the time left (wanted 20000s)",
        )

    def test_check_accepts_the_defaults_and_good_values_silently(self):
        for args in ((), ("--budget", "", "--note-s", ""), ("--budget", "600", "--note-s", "1"), ("--budget", "86400", "--note-s", "3600")):
            with self.subTest(args=args):
                p = run("check", *args)
                self.assertEqual((p.returncode, p.stdout), (0, b""), p.stderr)

    def test_check_refuses_each_bad_knob_by_name_on_stdout_with_status_2(self):
        for args, name in ((("--budget", "599"), b"KF_SPEC_COPY_BUDGET_S"), (("--budget", "x"), b"KF_SPEC_COPY_BUDGET_S"),
                           (("--budget", "86401"), b"KF_SPEC_COPY_BUDGET_S"), (("--note-s", "0"), b"KF_SPEC_COPY_NOTE_S"),
                           (("--note-s", "3601"), b"KF_SPEC_COPY_NOTE_S")):
            with self.subTest(args=args):
                p = run("check", *args)
                self.assertEqual(p.returncode, 2)
                self.assertIn(name, p.stdout)
                self.assertEqual(p.stdout.count(b"\n"), 1)


class Pipe(unittest.TestCase):
    def setUp(self):
        self._d = tempfile.TemporaryDirectory(prefix="kfcopy-")
        self.addCleanup(self._d.cleanup)
        self.dir = Path(self._d.name)
        self.status = self.dir / "status.txt"
        self.log = self.dir / "stage.log"

    def args(self, interval="30", budget="4200", expected="1000000", log=None):
        return ["pipe", "--status", str(self.status), "--log", str(log or self.log), "--budget", budget, "--expected", expected, "--interval", interval]

    def notes(self):
        return [l for l in self.log.read_text().splitlines() if l.startswith("SPEC_NOTE copy progress ")] if self.log.exists() else []

    def test_every_byte_passes_unchanged(self):
        data = bytes(range(256)) * 12000 + b"tail"
        p = run(*self.args(), stdin=data)
        self.assertEqual((p.returncode, p.stdout), (0, data))

    def test_an_empty_stream_passes_and_leaves_a_zero_byte_line(self):
        p = run(*self.args(), stdin=b"")
        self.assertEqual((p.returncode, p.stdout), (0, b""))
        self.assertEqual(len(self.notes()), 1)
        self.assertIn("bytes=0 rate_mb_s=0.0 ", self.notes()[0])
        self.assertTrue(self.notes()[0].endswith(" final=1"))

    def test_the_last_line_names_every_byte_and_the_budget(self):
        data = b"x" * 3_000_001
        run(*self.args(budget="777"), stdin=data)
        last = self.notes()[-1]
        self.assertIn("bytes=3000001 ", last)
        self.assertIn(" budget_s=777", last)
        self.assertTrue(last.endswith(" final=1"))
        self.assertTrue(last.startswith("SPEC_NOTE copy progress bytes=3000001 rate_mb_s="))

    def test_the_status_file_has_the_final_count(self):
        run(*self.args(), stdin=b"y" * 123456)
        n, elapsed, final = cb.read_status(self.status)
        self.assertEqual((n, final), (123456, True))
        self.assertGreaterEqual(elapsed, 0.0)

    def test_a_line_comes_every_interval_while_the_copy_runs(self):
        # a producer that sends 100 kB, waits 2.4 s, then sends the rest
        code = ("import sys,time; sys.stdout.buffer.write(b'a'*100000); sys.stdout.buffer.flush(); time.sleep(2.4); "
                "sys.stdout.buffer.write(b'b'*50000)")
        prod = subprocess.Popen([sys.executable, "-c", code], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        filt = subprocess.run([sys.executable, "-I", str(HELPER), *self.args(interval="0.5", budget="900")], stdin=prod.stdout, capture_output=True, timeout=60)
        prod.stdout.close()
        prod.wait()
        self.assertEqual(filt.returncode, 0, filt.stderr)
        self.assertEqual(len(filt.stdout), 150000)
        lines = self.notes()
        periodic = [l for l in lines if not l.endswith(" final=1")]
        self.assertGreaterEqual(len(periodic), 3, lines)
        for l in periodic:
            self.assertRegex(l, r"bytes=(0|100000) ")
            self.assertIn(" budget_s=900", l)
        self.assertGreaterEqual(len([l for l in periodic if "bytes=100000 " in l]), 2, lines)
        elapsed = [int(l.split("elapsed_s=")[1].split()[0]) for l in periodic]
        self.assertEqual(elapsed, sorted(elapsed))
        self.assertTrue(lines[-1].endswith(" final=1"))
        self.assertIn("bytes=150000 ", lines[-1])

    def test_a_stalled_copy_keeps_its_status_fresh_so_an_abort_can_name_the_bytes(self):
        # the consumer reads 70000 bytes and then stops reading; the producer has more
        code = "import sys,time; sys.stdout.buffer.write(b'z'*3000000); sys.stdout.buffer.flush()"
        prod = subprocess.Popen([sys.executable, "-c", code], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        filt = subprocess.Popen([sys.executable, "-I", str(HELPER), *self.args(interval="0.4")], stdin=prod.stdout, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        prod.stdout.close()
        got = filt.stdout.read(70000)
        self.assertEqual(len(got), 70000)
        time.sleep(2.2)  # the filter is now blocked in its write: the status must still tick
        n, elapsed, final = cb.read_status(self.status)
        self.assertFalse(final)
        self.assertGreaterEqual(n, 70000)
        self.assertLessEqual(n, 70000 + 3 * 65536)  # what the consumer took, the pipe in front of it and a chunk in flight
        self.assertGreaterEqual(elapsed, 1.5)
        self.assertGreaterEqual(len(self.notes()), 2)
        filt.stdout.close()  # the ssh was killed: the filter ends, quietly
        rc = filt.wait(timeout=30)
        prod.wait(timeout=30)
        self.assertNotEqual(rc, 0)
        err = filt.stderr.read()
        filt.stderr.close()
        self.assertNotIn(b"Traceback", err)
        self.assertTrue(cb.read_status(self.status)[2])

    def test_the_interval_comes_from_the_knob_text_when_no_override_is_given(self):
        code = "import sys,time; sys.stdout.buffer.write(b'a'*1000); sys.stdout.buffer.flush(); time.sleep(2.6)"
        prod = subprocess.Popen([sys.executable, "-c", code], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        args = ["pipe", "--status", str(self.status), "--log", str(self.log), "--budget", "900", "--expected", "1000", "--note-s", "1"]
        filt = subprocess.run([sys.executable, "-I", str(HELPER), *args], stdin=prod.stdout, capture_output=True, timeout=60)
        prod.stdout.close()
        prod.wait()
        self.assertEqual(filt.returncode, 0, filt.stderr)
        self.assertGreaterEqual(len([l for l in self.notes() if not l.endswith(" final=1")]), 2, self.notes())

    def test_a_bad_interval_knob_text_is_refused_by_the_pipe_before_it_reads_a_byte(self):
        args = ["pipe", "--status", str(self.status), "--log", str(self.log), "--budget", "900", "--expected", "1000", "--note-s", "0"]
        p = run(*args, stdin=b"data")
        self.assertEqual(p.returncode, 2)
        self.assertEqual(p.stdout, b"")
        self.assertIn(b"KF_SPEC_COPY_NOTE_S", p.stderr)

    def test_the_default_interval_is_60_seconds(self):
        # a stream that ends at once leaves only the last line: no periodic line can come inside 60 s
        args = ["pipe", "--status", str(self.status), "--log", str(self.log), "--budget", "900", "--expected", "1000"]
        p = run(*args, stdin=b"data")
        self.assertEqual((p.returncode, p.stdout), (0, b"data"))
        self.assertEqual(len(self.notes()), 1)
        self.assertEqual(cb.NOTE_DEFAULT_S, 60)

    def test_an_unwritable_log_never_fails_the_copy(self):
        data = b"q" * 400000
        p = run(*self.args(log=self.dir / "no" / "such" / "dir" / "log"), stdin=data)
        self.assertEqual((p.returncode, p.stdout), (0, data))

    def test_an_unwritable_status_file_never_fails_the_copy(self):
        data = b"q" * 400000
        args = self.args()
        args[args.index("--status") + 1] = str(self.dir / "no" / "dir" / "st")
        p = run(*args, stdin=data)
        self.assertEqual((p.returncode, p.stdout), (0, data))

    def test_the_abort_text_reads_the_status_file(self):
        self.status.write_text("COPY_PROGRESS bytes=1000000000 elapsed_s=200.0 final=0\n")
        p = run("abort-text", "--status", str(self.status), "--budget", "1000", "--expected", "43900000000")
        self.assertEqual(p.returncode, 0, p.stderr)
        text = p.stdout.decode("utf-8")
        self.assertIn("bytes=1000000000", text)
        self.assertIn("needed_s≈" + "8780", text)
        self.assertEqual(text.count("\n"), 1)

    def test_the_abort_text_without_a_status_file_says_nothing_arrived(self):
        p = run("abort-text", "--status", str(self.dir / "absent"), "--budget", "300", "--expected", "43900000000")
        text = p.stdout.decode("utf-8")
        self.assertEqual(p.returncode, 0)
        self.assertIn("bytes=0", text)
        self.assertIn("needed_s≈unknown", text)

    def test_a_garbled_status_file_reads_as_nothing_copied(self):
        self.status.write_text("not a status\n")
        self.assertEqual(cb.read_status(self.status), (0, 0.0, False))
        self.assertEqual(cb.read_status(self.dir / "absent"), (0, 0.0, False))


if __name__ == "__main__":
    unittest.main()
