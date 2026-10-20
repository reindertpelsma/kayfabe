#!/usr/bin/env python3
"""GPU-free tests for desktop_baton_parse.py (desktop-level baton classifier, tail gaps,
frame statistics, host status/event counters).

Run: python3 -m unittest scripts/bench/baton/test_desktop_baton_parse.py   (from the repo root)
 or: python3 test_desktop_baton_parse.py                                    (from this directory)
"""
import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import desktop_baton_parse as dbp  # noqa: E402


def it(idx, start, dur):
    return (idx, start, dur, start + dur / 1000.0)


def steady(t_from, t_to, hz, skip=()):
    """Completions at `hz` between t_from and t_to (seconds), minus any (lo, hi) skip ranges."""
    out, n, t = [], 0, t_from
    step = 1.0 / hz
    while t < t_to:
        if not any(lo <= t < hi for lo, hi in skip):
            out.append((n, t - 0.001, 1.0, t))
            n += 1
        t += step
    return out


def client(its, duration, t0=0.0, role="app"):
    return {"t0": t0, "its": its, "duration": duration, "role": role}


class ClientGaps(unittest.TestCase):
    def test_includes_leading_between_and_tail(self):
        its = [it(0, 1.0, 1000.0), it(1, 2.0, 1000.0)]  # done at 2.0 and 3.0
        gaps = dbp.client_gaps(its, duration_s=10)
        self.assertEqual(gaps, [(0.0, 2.0, False), (2.0, 3.0, False), (3.0, 10.0, True)])

    def test_no_completions_is_one_censored_gap(self):
        self.assertEqual(dbp.client_gaps([], duration_s=60), [(0.0, 60.0, True)])

    def test_tail_dropped_when_last_completion_at_window_end(self):
        self.assertEqual(dbp.client_gaps([it(0, 9.99, 10.0)], duration_s=10)[-1][2], False)


class Classify(unittest.TestCase):
    def test_no_events_when_steady(self):
        cl = {"A": client(steady(0, 60, 100), 60), "B": client(steady(0.005, 60, 100), 60)}
        self.assertEqual(dbp.classify(cl), [])

    def test_baton(self):
        cl = {"A": client(steady(0, 60, 100), 60),
              "B": client(steady(0, 60, 100, skip=[(20, 40)]), 60)}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["BATON"])
        e = ev[0]
        self.assertEqual(e["clients"], ["B"])
        self.assertAlmostEqual(e["from_s"], 19.99, places=1)
        self.assertAlmostEqual(e["gap_s"], 20.0, places=1)
        self.assertGreater(e["others"]["A"], 1000)
        self.assertFalse(e["censored"])
        self.assertIsNone(e["canary_alive"])

    def test_baton_with_canary_alive_flag(self):
        cl = {"A": client(steady(0, 60, 100), 60),
              "B": client(steady(0, 60, 100, skip=[(20, 40)]), 60),
              "C": client(steady(0, 60, 60), 60, role="canary")}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["BATON"])
        self.assertTrue(ev[0]["canary_alive"])

    def test_both_apps_stall_while_canary_alive(self):
        cl = {"A": client(steady(0, 60, 100, skip=[(10, 25)]), 60),
              "B": client(steady(0, 60, 100, skip=[(10, 25)]), 60),
              "C": client(steady(0, 60, 60), 60, role="canary")}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["APP_STALL_COMPOSITOR_ALIVE"])
        self.assertEqual(sorted(ev[0]["clients"]), ["A", "B"])

    def test_canary_silent_while_apps_run(self):
        cl = {"A": client(steady(0, 60, 100), 60), "B": client(steady(0, 60, 100), 60),
              "C": client(steady(0, 60, 60, skip=[(30, 40)]), 60, role="canary")}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["COMPOSITOR_STALL_APPS_ALIVE"])
        self.assertEqual(ev[0]["clients"], ["C"])

    def test_global_stall_is_one_merged_event(self):
        sk = [(30, 45)]
        cl = {"A": client(steady(0, 60, 100, skip=sk), 60), "B": client(steady(0, 60, 100, skip=sk), 60),
              "C": client(steady(0, 60, 60, skip=sk), 60, role="canary")}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["GLOBAL_STALL"])
        self.assertEqual(sorted(ev[0]["clients"]), ["A", "B", "C"])

    def test_global_stall_without_canary(self):
        sk = [(30, 45)]
        cl = {"A": client(steady(0, 60, 100, skip=sk), 60), "B": client(steady(0, 60, 100, skip=sk), 60)}
        self.assertEqual([e["kind"] for e in dbp.classify(cl)], ["GLOBAL_STALL"])

    def test_tail_starvation_is_a_censored_baton(self):
        # B stops at 40 s and never completes again; the run lasts 60 s.  baton_parse.py cannot
        # see this (no completion ends the gap); the tail gap is right-censored at the run end.
        cl = {"A": client(steady(0, 60, 100), 60), "B": client(steady(0, 40, 100), 60)}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["BATON"])
        self.assertTrue(ev[0]["censored"])
        self.assertAlmostEqual(ev[0]["to_s"], 60.0)

    def test_client_that_never_completes_is_starved(self):
        cl = {"A": client(steady(0, 60, 100), 60), "B": client([], 60)}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["BATON"])
        self.assertEqual(ev[0]["clients"], ["B"])
        self.assertTrue(ev[0]["censored"])

    def test_short_gaps_ignored(self):
        cl = {"A": client(steady(0, 60, 100), 60),
              "B": client(steady(0, 60, 100, skip=[(20, 23)]), 60)}
        self.assertEqual(dbp.classify(cl, min_gap_s=4.0), [])

    def test_clock_alignment(self):
        # B starts 10 s after A (t0); its own 5..15 s stall is 15..25 s on A's clock
        cl = {"A": client(steady(0, 60, 100), 60, t0=100.0),
              "B": client(steady(0, 50, 100, skip=[(5, 15)]), 50, t0=110.0)}
        ev = dbp.classify(cl)
        self.assertEqual([e["kind"] for e in ev], ["BATON"])
        self.assertAlmostEqual(ev[0]["from_s"], 15.0, places=1)

    def test_solo_stall(self):
        cl = {"A": client(steady(0, 60, 100, skip=[(10, 20)]), 60)}
        self.assertEqual([e["kind"] for e in dbp.classify(cl)], ["SOLO_STALL"])

    def test_verdict_priority(self):
        self.assertEqual(dbp.verdict([]), "NO_EVENT_OBSERVED")
        self.assertEqual(dbp.verdict([{"kind": "GLOBAL_STALL"}, {"kind": "BATON"}]), "BATON")
        self.assertEqual(dbp.verdict([{"kind": "GLOBAL_STALL"}]), "GLOBAL_STALL")
        self.assertEqual(dbp.verdict([{"kind": "COMPOSITOR_STALL_APPS_ALIVE"},
                                      {"kind": "APP_STALL_COMPOSITOR_ALIVE"}]),
                         "APP_STALL_COMPOSITOR_ALIVE")


class FrameStats(unittest.TestCase):
    def test_percentiles_and_gaps(self):
        # 100 completions 10 ms apart (0.01 .. 1.00 s), one 500 ms hole, window 3 s
        its = steady(0.01, 1.005, 100) + [(100, 1.0, 500.0, 1.5)]
        st = dbp.frame_stats(its, duration_s=3)
        self.assertEqual(st["frames"], 101)
        self.assertAlmostEqual(st["interval_p50_ms"], 10.0, places=0)
        self.assertAlmostEqual(st["interval_max_ms"], 500.0, places=0)
        self.assertAlmostEqual(st["tail_gap_ms"], 1500.0, places=0)
        self.assertAlmostEqual(st["longest_gap_incl_tail_ms"], 1500.0, places=0)
        self.assertEqual(st["zero_sec"], 1)  # second 2 is empty

    def test_empty(self):
        st = dbp.frame_stats([], duration_s=5)
        self.assertEqual(st["frames"], 0)
        self.assertEqual(st["zero_sec"], 5)
        self.assertAlmostEqual(st["longest_gap_incl_tail_ms"], 5000.0)


STATUS = ("kf3: family=Ada phase=Cold ... broker[up=1 sent=4821 gpucopy=4790 gpucopy_backoffs=0 "
          "dmabuf_trips=0 carrier_refused=0 carrier_unchecked=0 formats_unasked=0 dropped=3 "
          "uncommitted=1 recovered=0 releases=4818 unknown_releases=0 reclaims=0 blocked=2 "
          "backstops=0 reconnects=1 failed_attempts=0 peer_refused=0 cursor=compose cursor_sets=4 "
          "cursor_hides=0 cursor_shows=0 cursor_moves=0 cursor_refused=0] (BAR0 pieces=7)")


class HostCounters(unittest.TestCase):
    def test_fragment_parse_takes_last(self):
        text = "x broker[up=0 sent=0 dropped=0]\n" + STATUS + "\n"
        d = dbp.parse_status_fragments(text)
        self.assertEqual(d["broker"]["sent"], 4821)
        self.assertEqual(d["broker"]["dropped"], 3)
        self.assertEqual(d["broker"]["cursor"], "compose")
        self.assertEqual(d["broker"]["gpucopy"], 4790)

    def test_first_and_last_status(self):
        text = "a broker[up=1 sent=10 dropped=0]\nb broker[up=1 sent=500 dropped=2]\n"
        first, last = dbp.status_first_last(text, "broker")
        self.assertEqual((first["sent"], last["sent"]), (10, 500))

    def test_no_fragment(self):
        self.assertEqual(dbp.parse_status_fragments("nothing here"), {})
        self.assertEqual(dbp.status_first_last("nothing", "broker"), (None, None))

    def test_count_events(self):
        text = ("kf3: display: STALL 1.0 s\nkf3: display: STALL 2.0 s\nfoo Exception bar\n"
                "kf3: scanout REFUSED x\nkf3: cursor REFUSED y\nkf3: cursor REFUSED z\nkf3: broker: hello\n")
        c = dbp.count_events(text)
        self.assertEqual(c, {"stall": 2, "exception": 1, "scanout_refused": 1, "cursor_refused": 2,
                             "broker_lines": 1})


def baton_log(label, duration, t0):
    return ("# baton v1\n"
            "# label=%s pid=1 k_ms=5.000 cycles=1000 grid=900x700 duration_s=%d\n"
            "# t0_epoch=%.6f\nsec iterations max_iter_ms\n" % (label, duration, t0))


def dump_csv(its):
    return "idx,start_s,dur_ms,done_s\n" + "".join("%d,%.6f,%.3f,%.6f\n" % i for i in its)


class Cli(unittest.TestCase):
    def test_report_end_to_end(self):
        with tempfile.TemporaryDirectory() as d:
            specs = []
            for lab, its, role in (("A", steady(0, 30, 100), "app"),
                                   ("B", steady(0, 30, 100, skip=[(10, 20)]), "app"),
                                   ("C", steady(0, 30, 60), "canary")):
                lp, dp = os.path.join(d, lab + ".log"), os.path.join(d, lab + ".csv")
                open(lp, "w").write(baton_log(lab, 30, 1000.0))
                open(dp, "w").write(dump_csv(its))
                specs.append(("--canary" if role == "canary" else "--client", "%s:%s" % (lp, dp)))
            qlog = os.path.join(d, "qemu.log")
            open(qlog, "w").write(STATUS + "\nkf3: display: STALL 3 s\n")
            argv = ["report"]
            for flag, spec in specs:
                argv += [flag, spec]
            argv += ["--qemu", qlog]
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = dbp.main(argv)
            out = buf.getvalue()
            self.assertEqual(rc, 0)
            self.assertIn("DESKTOP_BATON_VERDICT=BATON", out)
            self.assertIn("BATON", out)
            self.assertIn("canary_alive=True", out)
            self.assertIn("host broker sent=4821", out)
            self.assertIn("display_stall=1", out)

    def test_report_without_canary_or_qemu(self):
        with tempfile.TemporaryDirectory() as d:
            lp, dp = os.path.join(d, "A.log"), os.path.join(d, "A.csv")
            open(lp, "w").write(baton_log("A", 10, 5.0))
            open(dp, "w").write(dump_csv(steady(0, 10, 100)))
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = dbp.main(["report", "--client", "%s:%s" % (lp, dp)])
            self.assertEqual(rc, 0)
            self.assertIn("DESKTOP_BATON_VERDICT=NO_EVENT_OBSERVED", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
