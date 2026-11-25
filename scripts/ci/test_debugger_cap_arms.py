"""GPU-free tests of scripts/bench/debugger_cap_arms.py: the parsing, the stop and leak rules, the
preregistered cap derivation, and one real arm lifecycle against a fake worker that speaks the probe's
protocol (go / stop / release / exit)."""

import importlib.util
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SRC = Path(__file__).resolve().parents[1] / "bench" / "debugger_cap_arms.py"
_spec = importlib.util.spec_from_file_location("debugger_cap_arms", SRC)
arms = importlib.util.module_from_spec(_spec)
sys.modules["debugger_cap_arms"] = arms
_spec.loader.exec_module(arms)

FAKE_WORKER = textwrap.dedent(
    '''
    import select, sys, time
    REFUSE_AT = {refuse_at}
    args = sys.argv[1:]
    maxn = int(args[args.index("--max-sessions") + 1])
    every = int(args[args.index("--progress-every") + 1])

    def readline_nowait():
        r, _, _ = select.select([sys.stdin], [], [], 0)
        return sys.stdin.readline().strip() if r else None

    def readline_wait(want):
        while True:
            ln = sys.stdin.readline()
            if not ln or ln.strip() in want or ln.strip() == "exit":
                return ln.strip() or "exit"

    print("SETUP driver=0.0 grs_built=1 grs_asked=1 ms=1 rss_kib=1000", flush=True)
    print("READY", flush=True)
    readline_wait({{"go"}})
    n = 0
    reason = "ceiling"
    while True:
        if readline_nowait() == "stop":
            reason = "stop"; break
        if n >= maxn:
            break
        if REFUSE_AT and n >= REFUSE_AT:
            print("REFUSED gr=0 on_gr=%d total=%d after_ms=1 err=NV_ERR_NO_MEMORY(0x51) raw=NoMemory" % (n, n), flush=True)
            reason = "all_refused"; break
        n += 1
        if n % every == 0:
            print("PROGRESS n=%d t_ms=%d rss_kib=1000 win_mean_us=5 win_p99_us=9 win_max_us=20" % (n, n), flush=True)
            time.sleep(0.02)
    print("ALLOC_DONE reason=%s total=%d refused_grs=0 grs=1 per_gr_min=%d per_gr_max=%d ms=1 rss_kib=1 mean_us=5 p50_us=5 p99_us=9 max_us=20" % (reason, n, n, n), flush=True)
    cmd = readline_wait({{"release"}})
    print("FREE_DONE freed=%d failed=0 ms=1 first_failure=-" % n, flush=True)
    if cmd != "exit":
        readline_wait(set())
    print("EXIT rc=0", flush=True)
    '''
)


class FakeSources:
    def __init__(self, grow_per_call=0, leak_after_calls=None, xid_after_calls=None):
        self.calls = 0
        self.dcalls = 0
        self.grow = grow_per_call
        self.leak_after = leak_after_calls
        self.xid_after = xid_after_calls

    def fb_mib(self):
        self.calls += 1
        leak = 50 if self.leak_after is not None and self.calls > self.leak_after else 0
        return 1000 + self.grow * max(0, self.calls - 4) + leak

    def meminfo(self):
        return {"MemAvailable": 5000, "SUnreclaim": 100, "Slab": 200}

    def dmesg(self):
        self.dcalls += 1
        extra = ""
        if self.xid_after is not None and self.dcalls > self.xid_after:
            extra = "[ 5.0] NVRM: Xid (PCI:0000:01:00): 31, fake\n"
        return "[ 1.0] NVRM: loaded GPU-12345678-1234-1234-1234-123456789abc at 0000:01:00.0\n" + extra


def run(tmp, refuse_at=0, ceiling=300, src=None, fb_limit=4096, settle=0.2, procs=1):
    probe = Path(tmp) / "fake_probe.py"
    probe.write_text(FAKE_WORKER.format(refuse_at=refuse_at))
    sh = Path(tmp) / "probe"
    sh.write_text("#!/bin/sh\nexec %s %s \"$@\"\n" % (sys.executable, probe))
    sh.chmod(0o755)
    return arms.run_arm("t", (procs, 1, 0), sh, Path(tmp) / "out", src or FakeSources(), ceiling, fb_limit, 30,
                        settle, 2, 0, sample_s=0.02, progress_every=50, pause_s=0.05)


class Pure(unittest.TestCase):
    def test_parse_line(self):
        self.assertEqual(arms.parse_line("PROGRESS n=100 t_ms=5"), ("PROGRESS", {"n": "100", "t_ms": "5"}))
        self.assertEqual(arms.parse_line("READY"), ("READY", {}))
        self.assertIsNone(arms.parse_line("  \n"))

    def test_meminfo_and_xids(self):
        m = arms.parse_meminfo("MemTotal:  100 kB\nSlab:   7 kB\nSUnreclaim:  3 kB\n")
        self.assertEqual((m["MemTotal"], m["Slab"], m["SUnreclaim"]), (100, 7, 3))
        self.assertEqual(arms.count_xids("a\nNVRM: Xid (PCI:0): 31\nNVRM: loaded\nNVRM: Xid 13\n"), 2)

    def test_scrub_hides_identity(self):
        s = arms.scrub("NVRM: GPU-12345678-1234-1234-1234-123456789abc at PCI 0000:01:00.0 ok")
        self.assertNotIn("GPU-1234", s)
        self.assertNotIn("0000:01:00.0", s)
        self.assertIn("<uuid>", s)
        self.assertIn("<bdf>", s)

    def test_leak_and_limits(self):
        self.assertEqual(arms.leak_verdict(1000, 1002, 2), "ok")
        self.assertEqual(arms.leak_verdict(1000, 1003, 2), "LEAK +3 MiB")
        self.assertEqual(arms.leak_verdict(1000, 900, 2), "ok")
        self.assertTrue(arms.fb_limit_hit(1000, 5096, 4096))
        self.assertFalse(arms.fb_limit_hit(1000, 5095, 4096))
        self.assertEqual(arms.kib_per_session(100, 101, 1024), 1.0)
        self.assertIsNone(arms.kib_per_session(100, 101, 0))

    def test_recommend_follows_the_preregistered_rule(self):
        r = arms.recommend(10000, None)
        self.assertEqual((r["cap"], r["vms_default"], r["vms_min"]), (2496, 1, 9))
        r = arms.recommend(None, 64.0)  # 1 GiB / 64 KiB = 16384, a multiple of 64
        self.assertEqual((r["cap"], r["vms_default"]), (16384, 9))
        r = arms.recommend(None, 4096.0)  # 256 sessions per GiB: never below one default VM
        self.assertEqual(r["cap"], arms.DEFAULT_BUDGET)
        self.assertIsNone(arms.recommend(None, None)["cap"])

    def test_arms_table_matches_the_preregistration(self):
        self.assertEqual(arms.ARMS["a1"], (1, 1, 0))
        self.assertEqual(arms.ARMS["a5"], (16, 1, 0))
        self.assertEqual(arms.ARMS["k2"], (1, 448, 4))
        self.assertEqual(sorted(arms.ARMS), ["a1", "a2", "a3", "a4", "a5", "k1", "k2"])


class Lifecycle(unittest.TestCase):
    def test_ceiling_stops_the_arm_and_everything_is_freed(self):
        with tempfile.TemporaryDirectory() as tmp:
            s = run(tmp, ceiling=300, procs=2)
            self.assertEqual(s["stop_cause"], "ceiling")
            self.assertGreaterEqual(s["sessions_total"], 300)
            self.assertTrue(s["free_ok"])
            self.assertEqual(s["leak_after_exit_vs_baseline"], "ok")
            self.assertEqual(s["new_xids"], 0)
            self.assertEqual(len(s["workers"]), 2)

    def test_refusal_is_recorded_and_ends_the_arm(self):
        with tempfile.TemporaryDirectory() as tmp:
            s = run(tmp, refuse_at=120, ceiling=100000)
            self.assertEqual(s["stop_cause"], "workers_done")
            self.assertEqual(s["sessions_total"], 120)
            self.assertEqual(s["refusing_workers"], 1)
            self.assertIn("0x51", s["workers"][0]["refusals"][0]["err"])
            self.assertEqual(s["workers"][0]["alloc_done"]["reason"], "all_refused")

    def test_an_xid_stops_the_arm(self):
        with tempfile.TemporaryDirectory() as tmp:
            s = run(tmp, ceiling=10**9, src=FakeSources(xid_after_calls=2))
            self.assertEqual(s["stop_cause"], "xid")
            self.assertEqual(s["new_xids"], 1)
            text = (Path(tmp) / "out" / "t_dmesg.txt").read_text()
            self.assertIn("Xid", text)
            self.assertNotIn("GPU-1234", text)

    def test_fb_growth_stops_the_arm(self):
        with tempfile.TemporaryDirectory() as tmp:
            s = run(tmp, ceiling=10**9, src=FakeSources(grow_per_call=100), fb_limit=500)
            self.assertEqual(s["stop_cause"], "fb_limit")

    def test_a_leak_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            s = run(tmp, ceiling=100, src=FakeSources(leak_after_calls=1), settle=0.2)
            self.assertTrue(s["leak_after_exit_vs_baseline"].startswith("LEAK"))
            self.assertEqual(s["leak_after_free_vs_setup"], "ok")


if __name__ == "__main__":
    unittest.main()
