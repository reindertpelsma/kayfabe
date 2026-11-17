# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""DirectX-Graphics-Samples harness (dxs_harness.py): pixel logic, the ReservedResources title model, the windowed run with a fake window and clock, the
console sample judge. GPU-free; the ctypes Win32 layer is not exercised (it never ran on Windows)."""
import contextlib
import io
import unittest

import helpers as H
import dxs_harness as D
import kfopen_pins as P
from kfopen_common import OpenRowError

PINS = P.load_pins()
ROWS = PINS["rows"]


def mip_title(first):
    """The title function of a fake ReservedResources: starts at `first`, UP/DOWN move like the sample (UP 0x26 toward 0, DOWN 0x28 toward first)."""
    state = {"mip": first, "n": 0}

    def fn(keys):
        while state["n"] < len(keys):
            vk = keys[state["n"]]
            state["n"] += 1
            if vk == D.VK["UP"]:
                state["mip"] = max(state["mip"] - 1, 0)
            elif vk == D.VK["DOWN"]:
                state["mip"] = min(state["mip"] + 1, first)
        return "D3D12 Reserved Resources Sample - Mip Level: %d" % state["mip"]
    return fn


def drive(spec, captures, exit_at=None, rc=0, appear_s=1.0, title_fn=None, exit_on_close=True):
    clock = H.FakeClock()
    proc = H.FakeProc(clock, exit_at=exit_at, rc=rc, exit_on_close=exit_on_close)
    win = H.FakeWin(clock, proc, appear_s=appear_s, title_fn=title_fn, captures=captures)
    facts = D.run_window_sample(spec, proc, win, clock)
    return facts, proc, win


def names(checks, ok):
    return [n for n, o, _ in checks if o == ok]


class Pixels(unittest.TestCase):
    def test_busy_capture_is_nonblank_and_hashed(self):
        w, h, px = H.busy_capture(1)
        a = D.analyze_pixels(px, w, h)
        self.assertTrue(a["nonblank"])
        self.assertEqual(64, len(a["sha256"]))

    def test_uniform_pixels_not_nonblank(self):
        w, h, px = H.blank_capture()
        a = D.analyze_pixels(px, w, h)
        self.assertFalse(a["nonblank"])
        self.assertEqual(1, a["unique"])

    def test_blank_capture_fails_check(self):
        spec = dict(ROWS["dxsamples_hdr"])
        facts, _, _ = drive(spec, [H.blank_capture()])
        self.assertIn("screenshot_nonblank", names(D.judge_window(spec, facts), False))

    def test_hash_changes_with_the_pixels_not_with_time(self):
        a = D.analyze_pixels(H.busy_capture(1)[2], 96, 96)
        b = D.analyze_pixels(H.busy_capture(1)[2], 96, 96)
        c = D.analyze_pixels(H.busy_capture(2)[2], 96, 96)
        self.assertEqual(a["sha256"], b["sha256"])
        self.assertNotEqual(a["sha256"], c["sha256"])

    def test_capture_shape_mismatch_refused(self):
        with self.assertRaisesRegex(OpenRowError, "CAPTURE_SHAPE"):
            D.analyze_pixels(b"\x00" * 100, 96, 96)

    def test_tiny_window_refused(self):
        with self.assertRaisesRegex(OpenRowError, "CAPTURE_SHAPE"):
            D.analyze_pixels(b"\x00" * (32 * 32 * 4), 32, 32)


class Keys(unittest.TestCase):
    def test_vk_codes(self):
        self.assertEqual({"SPACE": 0x20, "PRIOR": 0x21, "NEXT": 0x22, "UP": 0x26, "DOWN": 0x28, "H": 0x48, "U": 0x55}, D.VK)

    def test_expand_keys_repeats(self):
        self.assertEqual(["UP"] * 3 + ["DOWN"], D.expand_keys([{"vk": "UP", "repeat": 3}, {"vk": "DOWN"}]))

    def test_unknown_key_refused(self):
        with self.assertRaises(OpenRowError):
            D.expand_keys([{"vk": "F13"}])

    def test_mip_model_clamps_at_zero(self):
        self.assertEqual([2, 1, 0, 0, 0], D.expected_mip_titles(3, ["UP"] * 5))

    def test_title_sequence_with_clamped_keys(self):
        self.assertEqual([3, 3, 2, 3, 3], D.expected_mip_titles(3, ["DOWN", "DOWN", "UP", "DOWN", "DOWN"]))
        self.assertEqual([2, 1, 0, 0, 1], D.expected_mip_titles(3, ["UP", "UP", "UP", "UP", "DOWN"]))

    def test_mip_title_regex(self):
        self.assertEqual(5, D.mip_from_title("x - Mip Level: 5", ROWS["dxsamples_reserved_resources"]["title_regex"]))
        self.assertIsNone(D.mip_from_title("x", ROWS["dxsamples_reserved_resources"]["title_regex"]))

    def test_every_key_of_the_rows_is_known(self):
        for rid, row in ROWS.items():
            if row["kind"] == "dx-window":
                D.expand_keys(row["keys"])

    def test_reserved_resources_walks_the_whole_mip_chain_both_ways(self):
        keys = D.expand_keys(ROWS["dxsamples_reserved_resources"]["keys"])
        self.assertEqual(24, len(keys), "24 UpdateTileMappings steps")
        seq = D.expected_mip_titles(8, keys)
        self.assertEqual(0, min(seq))
        self.assertEqual(8, max(seq))


class WindowRun(unittest.TestCase):
    def good(self, rid):
        spec = ROWS[rid]
        facts, proc, win = drive(spec, [H.busy_capture(1), H.busy_capture(2)], title_fn=mip_title(8) if spec.get("title_regex") else None)
        return spec, facts, proc, win

    def test_animated_sample_passes_every_check(self):
        spec, facts, proc, win = self.good("dxsamples_nbody_async")
        checks = D.judge_window(spec, facts)
        self.assertEqual([], names(checks, False), checks)
        self.assertTrue(win.closed)
        self.assertEqual(0, facts["close"]["rc"])

    def test_keys_posted_with_vk_codes(self):
        spec, facts, proc, win = self.good("dxsamples_execute_indirect")
        self.assertEqual([0x20, 0x20], win.keys)
        spec, facts, proc, win = self.good("dxsamples_hdr")
        self.assertEqual([0x21, 0x21, 0x22, 0x22, 0x48, 0x48], win.keys)

    def test_reserved_resources_title_sequence_passes(self):
        spec, facts, _, win = self.good("dxsamples_reserved_resources")
        checks = D.judge_window(spec, facts)
        self.assertEqual([], names(checks, False), checks)
        self.assertEqual(24, len(win.keys))

    def test_the_first_title_is_awaited_for_up_to_twenty_seconds(self):
        spec = ROWS["dxsamples_reserved_resources"]
        clock = H.FakeClock()
        proc = H.FakeProc(clock)
        inner = mip_title(8)
        late = lambda keys: ("Sample - " + inner(keys)) if clock.now() >= 7.0 else "D3D12 Reserved Resources Sample"
        win = H.FakeWin(clock, proc, title_fn=late, captures=[H.busy_capture(1), H.busy_capture(2)])
        facts = D.run_window_sample(spec, proc, win, clock)
        self.assertEqual([], names(D.judge_window(spec, facts), False), D.judge_window(spec, facts))

    def test_a_title_that_never_appears_is_judged_after_the_wait(self):
        spec = ROWS["dxsamples_reserved_resources"]
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], title_fn=lambda keys: "D3D12 Reserved Resources Sample")
        self.assertIn("title_sequence", names(D.judge_window(spec, facts), False))

    def test_wrong_mip_sequence_fails(self):
        spec = ROWS["dxsamples_reserved_resources"]
        stuck = lambda keys: "Sample - Mip Level: 8"
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], title_fn=stuck)
        self.assertIn("title_sequence", names(D.judge_window(spec, facts), False))

    def test_missing_title_fails(self):
        spec = ROWS["dxsamples_reserved_resources"]
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], title_fn=lambda keys: "D3D12 Reserved Resources Sample")
        checks = D.judge_window(spec, facts)
        self.assertIn("title_sequence", names(checks, False))
        self.assertIn("tier unsupported or stuck", [w for n, o, w in checks if n == "title_sequence"][0])

    def test_identical_captures_fail_frames_advance(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, _ = drive(spec, [H.busy_capture(1)])
        self.assertIn("frames_advance", names(D.judge_window(spec, facts), False))

    def test_judge_requires_frames_advance_for_animated_rows(self):
        self.assertTrue(ROWS["dxsamples_nbody_async"]["frames_advance"])
        self.assertTrue(ROWS["dxsamples_execute_indirect"]["frames_advance"])
        spec = dict(ROWS["dxsamples_nbody_async"], frames_advance=False)
        facts, _, _ = drive(spec, [H.busy_capture(1)])
        self.assertNotIn("frames_advance", [n for n, _, _ in D.judge_window(spec, facts)])

    def test_early_exit_fails(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, _ = drive(spec, [H.busy_capture(1)], exit_at=10.0, rc=1)
        checks = D.judge_window(spec, facts)
        self.assertIn("sample_started_window", names(checks, False))
        self.assertEqual(1, facts["early_exit_rc"])

    def test_early_exit_before_window(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, _ = drive(spec, [H.busy_capture(1)], exit_at=0.2, rc=3, appear_s=5.0)
        checks = D.judge_window(spec, facts)
        self.assertIn("exited rc=3 before it drew a window", [w for n, o, w in checks if n == "sample_started_window"][0])
        self.assertEqual(1, len(checks), "nothing else is judged without a window")

    def test_no_window_ever_kills_the_process_and_fails(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, proc, _ = drive(spec, [H.busy_capture(1)], appear_s=10 ** 6)
        self.assertTrue(proc.killed)
        self.assertIn("sample_started_window", names(D.judge_window(spec, facts), False))

    def test_hung_window_fails_close(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, proc, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], exit_on_close=False)
        checks = D.judge_window(spec, facts)
        self.assertIn("close_graceful", names(checks, False))
        self.assertTrue(proc.killed)

    def test_killed_process_not_graceful(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], exit_on_close=False)
        self.assertFalse(facts["close"]["exited"])
        self.assertIn("close_graceful", names(D.judge_window(spec, facts), False))

    def test_exit_after_drawing_is_still_an_early_exit(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, _ = drive(spec, [H.busy_capture(1)], exit_at=20.0, rc=0)
        self.assertIn("sample_started_window", names(D.judge_window(spec, facts), False))

    def test_title_sequence_after_every_key_is_compared(self):
        spec = ROWS["dxsamples_reserved_resources"]
        off_by_one = lambda keys: "Mip Level: %d" % max(8 - len(keys) + 1, 0)
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], title_fn=off_by_one)
        self.assertIn("title_sequence", names(D.judge_window(spec, facts), False))

    def test_exit_code_check_is_reported(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], rc=7)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = D.report("dxsamples_nbody_async", spec, D.judge_window(spec, facts), facts)
        self.assertEqual(1, rc)
        self.assertIn("CHECK exit_code_zero FAIL exit code 7", buf.getvalue())

    def test_nonzero_exit_fails(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], rc=5)
        self.assertIn("exit_code_zero", names(D.judge_window(spec, facts), False))

    def test_run_window_sample_records_exit(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, proc, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)], rc=7)
        self.assertEqual({"exited": True, "rc": 7}, {k: facts["close"][k] for k in ("exited", "rc")})

    def test_two_captures_are_taken_even_by_a_short_run(self):
        spec = dict(ROWS["dxsamples_hdr"], seconds=3)
        facts, _, _ = drive(spec, [H.busy_capture(1), H.busy_capture(2)])
        self.assertEqual(2, len(facts["captures"]))

    def test_run_lasts_the_asked_seconds(self):
        spec = ROWS["dxsamples_nbody_async"]
        facts, _, win = drive(spec, [H.busy_capture(1), H.busy_capture(2)])
        self.assertGreaterEqual(facts["captures"][-1]["t"], spec["seconds"] / 2 - 1)

    def test_report_prints_checks_and_a_verdict_line_and_returns_the_exit_code(self):
        spec, facts, _, _ = self.good("dxsamples_execute_indirect")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = D.report("dxsamples_execute_indirect", spec, D.judge_window(spec, facts), facts)
        self.assertEqual(0, rc)
        self.assertIn("KFOPEN VERDICT PASS row=dxsamples_execute_indirect keys=2", buf.getvalue())
        self.assertIn("KFOPEN WINDOW_HASH", buf.getvalue())
        spec = dict(spec, frames_advance=True)
        facts, _, _ = drive(spec, [H.busy_capture(1)])
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = D.report("dxsamples_execute_indirect", spec, D.judge_window(spec, facts), facts)
        self.assertEqual(1, rc)
        self.assertIn("CHECK frames_advance FAIL", buf.getvalue())
        self.assertIn("KFOPEN VERDICT FAIL row=dxsamples_execute_indirect failed=frames_advance", buf.getvalue())


UAV_OK = ">>> Dumping 8 uints from UAV:\n" + "".join("    UAV[%d] = 0x%x\n" % (i, 4 + i) for i in range(8)) + "\n Execution complete\n"


class Console(unittest.TestCase):
    SPEC = ROWS["dxsamples_work_graphs"]

    def test_good_run_passes(self):
        checks, status, uav = D.judge_console(self.SPEC, 0, UAV_OK)
        self.assertEqual("ok", status)
        self.assertEqual(8, len(uav))

    def test_console_uav_count(self):
        text = UAV_OK.replace("    UAV[7] = 0xb\n", "")
        checks, status, _ = D.judge_console(self.SPEC, 0, text)
        self.assertEqual("fail", status)
        self.assertIn("uav_lines", names(checks, False))

    def test_console_extra_uav_lines_fail(self):
        text = UAV_OK + "    UAV[8] = 0x1\n"
        checks, status, _ = D.judge_console(self.SPEC, 0, text)
        self.assertIn("uav_lines", names(checks, False))

    def test_console_all_zero_fails(self):
        text = "".join("    UAV[%d] = 0x0\n" % i for i in range(8)) + "Execution complete\n"
        checks, status, _ = D.judge_console(self.SPEC, 0, text)
        self.assertIn("uav_written", names(checks, False))

    def test_nonzero_exit_and_missing_completion_fail(self):
        checks, status, _ = D.judge_console(self.SPEC, -1, UAV_OK)
        self.assertIn("exit_code_zero", names(checks, False))
        checks, status, _ = D.judge_console(self.SPEC, 0, UAV_OK.replace("Execution complete", "Aborting."))
        self.assertIn("execution_complete", names(checks, False))

    def test_work_graphs_unsupported_is_notrun(self):
        checks, status, _ = D.judge_console(self.SPEC, -1, "Device does not report support for work graphs.")
        self.assertEqual("notrun", status)

    def test_notsupported_checked_before_exit_code(self):
        _, status, _ = D.judge_console(self.SPEC, 0, "Device does not report support for work graphs.\n" + UAV_OK)
        self.assertEqual("notrun", status)

    def test_console_differs_from_control_fails(self):
        control = [(i, 4 + i) for i in range(8)]
        other = [(i, 5 + i) for i in range(8)]
        checks, status, _ = D.judge_console(self.SPEC, 0, UAV_OK, control=other)
        self.assertIn("equals_control", names(checks, False))
        self.assertEqual("ok", D.judge_console(self.SPEC, 0, UAV_OK, control=control)[1])

    def test_console_control_of_another_length_fails(self):
        checks, status, _ = D.judge_console(self.SPEC, 0, UAV_OK, control=[(0, 4)])
        self.assertEqual("fail", status)


if __name__ == "__main__":
    unittest.main()
