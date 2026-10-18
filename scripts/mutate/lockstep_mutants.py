#!/usr/bin/env python3
"""Mutation runner for the phase-2 lock step (docs/design/V3_DISPLAY_PACING_20261017.md section 11.7).

Each mutant is ONE textual replacement in ONE source file; the runner applies it, runs the named
test suite(s), records killed/survived, and restores the file (git checkout of that file is the
safety net: `git status` must be clean afterwards).

    scripts/mutate/lockstep_mutants.py --suite a|b|threads|relay|all [--only NAME] [--list]

Suites (test-name filters of `cargo test --release`):
  a        kf-disp --lib   lockstep_exhaustive      (the exhaustive bounded model)
  b        kf-disp --lib   lockstep_soak            (the seeded soak, real Engine/relay/ring)
  threads  kf-disp --test lockstep_threads          (real threads, real socket, fake broker)
  relay    kf-broker --test relay_ticks + --lib ticks:: (the relay's own tests)
  unit     kf-disp --lib tick:: lockstep:: pace::  (the pure units)
`--suite must` runs each mutant against the suites of its must-list. A mutant a required suite fails to kill is printed as SURVIVED and the exit code is 1.
"""
import argparse, os, subprocess, sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
TARGET = os.environ.get("CARGO_TARGET_DIR", "/workspace/kf-pacing2-target")

SUITES = {
    "a": ["cargo", "test", "--release", "-p", "kf-disp", "--lib", "lockstep_exhaustive"],
    "b": ["cargo", "test", "--release", "-p", "kf-disp", "--lib", "lockstep_soak"],
    "threads": ["cargo", "test", "--release", "-p", "kf-disp", "--test", "lockstep_threads"],
    "relay": ["cargo", "test", "--release", "-p", "kf-broker"],
    "unit": ["cargo", "test", "--release", "-p", "kf-disp", "--lib", "--", "tick::", "lockstep::", "pace::"],
}

B = "crates/kf-broker/src/"
D = "crates/kf-disp/src/"
# name, file, old, new, which suites MUST kill it
MUTANTS = [
    ("credit_check_removed", B + "ticks.rs",
     "        if open.is_some() {\n            FrameGate::Send", "        if true {\n            FrameGate::Send", "a b relay"),
    ("request_not_replaced (coalescing)", B + "ticks.rs",
     "            self.open = Some(t.tick_seq);\n            self.ticks_since_open = 0;",
     "            self.open = self.open.or(Some(t.tick_seq));\n            self.ticks_since_open = 0;", "a b relay"),
    ("credit_not_consumed (gate)", D + "lockstep.rs",
     "        self.pending = None;\n        self.c.granted += 1;", "        self.c.granted += 1;", "a b unit"),
    ("worker_fallback_removed", D + "tick.rs",
     "                if now_ns < self.deadline_ns() {\n                    return (None, None);\n                }\n                self.source = Source::Free;",
     "                if true {\n                    return (None, None);\n                }\n                self.source = Source::Free;", "a b unit"),
    ("relay_silence_removed", B + "ticks.rs",
     "now.saturating_sub(r) <= BROKER_SILENCE_MS)", "now.saturating_sub(r) <= u64::MAX)", "a b relay"),
    ("buffer_reuse_rule_removed", D + "pace.rs",
     "    floor.max(inflight.unwrap_or(0))", "    floor", "a b unit"),
    ("relay_sequence_check_removed", B + "ticks.rs",
     "        if self.last_seq.is_some_and(|l| !seq_newer(t.tick_seq, l)) {", "        if false {", "a b relay"),
    ("gate_sequence_check_removed", D + "lockstep.rs",
     "        if self.last_seq.is_some_and(|l| !seq_newer(tick_seq, l)) {", "        if false {", "a b unit"),
    ("in_flight_bound_raised", B + "slots.rs",
     "pub const HELD_CAP: u32 = 2;", "pub const HELD_CAP: u32 = 4;", "a b relay"),
    ("double_tick_guard_removed", D + "tick.rs",
     "            .is_some_and(|t| now_ns.saturating_sub(t) < guard)", "            .is_some_and(|t| now_ns.saturating_sub(t) < 0)", "a b unit"),
    ("swallowed_credit_acts_at_once (guard defer removed)", D + "lockstep.rs",
     "                self.c.credits_deferred += 1;\n                AWAIT_TICK", "                self.c.credits_deferred += 1;\n                0", "a b unit"),
    ("carried_credit_double_counts (copies > vblanks)", D + "lockstep.rs",
     "        if let (Some(stamp), Some((r, AWAIT_TICK))) = (emitted, self.pending) {\n            self.pending = Some((r, stamp));\n        }",
     "", "a b unit"),
    ("gate_acts_early", D + "lockstep.rs",
     "        if now_ns < at {\n            return None;\n        }", "        if false {\n            return None;\n        }", "a b unit"),
    ("flap_damping_removed", D + "tick.rs",
     "if self.source == Source::Free && self.fallbacks_run > 0 {", "if self.source == Source::Free && false {", "a b unit"),
    ("no_change_at_the_opening_tick", B + "ticks.rs",
     "            nothing_to_send\n                && self.ticks_since_open >= 1", "            nothing_to_send\n                && self.ticks_since_open >= 0", "a b relay"),
    ("read_deadline_removed", B + "conn.rs",
     ".is_some_and(|t| now >= t + PARTIAL_READ_MS)", ".is_some_and(|_| false)", "b threads relay"),
    ("per_turn_budget_removed", B + "conn.rs",
     "            self.read_batch(now, host, out, cap.clamp(1, READ_BATCH));",
     "            for _ in 0..4096 {\n                self.read_batch(now, host, out, cap.clamp(1, READ_BATCH));\n            }", "b threads relay"),
    ("send_blocks_on_full", B + "conn.rs",
     "        self.link.send(&c.sock, &rec, fd)\n    }",
     "        let mut r = self.link.send(&c.sock, &rec, fd);\n        while r == Sent::Full {\n            r = self.link.send(&c.sock, &rec, fd);\n        }\n        r\n    }", "b threads relay"),
    ("stale_tick_counts_as_alive", B + "conn.rs",
     "            TickVerdict::Stale => {}", "            TickVerdict::Stale => {\n                tc.last_rx_ms = Some(now);\n            }", "a b relay"),
    ("want_lost_on_coalesce (share)", B + "ticks.rs",
     "        if t.want && self.want.swap(t.tick_seq, Ordering::AcqRel) != 0 {", "        if t.want && self.want.load(Ordering::Acquire) != 0 {", "b threads relay"),
]


def run(cmd):
    env = dict(os.environ, CARGO_TARGET_DIR=TARGET)
    r = subprocess.run(cmd + (["-j2"] if "--" not in cmd else []), cwd=ROOT, env=env,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return r.returncode, r.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--suite", default="all")
    ap.add_argument("--only")
    ap.add_argument("--part", help="i/n: run every n-th mutant starting at i (parallel workers)")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()
    extra = []
    for f in sorted(os.listdir(os.path.dirname(os.path.abspath(__file__)))):
        if f.startswith("lockstep_extra_") and f.endswith(".py"):
            ns = {}
            exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), f)).read(), ns)
            extra += ns.get("MUTANTS", [])
    muts = MUTANTS + extra
    if a.list:
        for m in muts:
            print(m[0], "->", m[4])
        return 0
    bad = 0
    print(f"{'mutant':40} " + " ".join(f"{s:8}" for s in (["a", "b", "threads", "relay", "unit"] if a.suite == "all" else [a.suite])))
    if a.suite == "must":
        print("(suite 'must': each mutant is run against the suites its must-list names, in that order)")
    for idx, (name, rel, old, new, must) in enumerate(muts):
        if a.only and a.only not in name:
            continue
        if a.part and idx % int(a.part.split("/")[1]) != int(a.part.split("/")[0]):
            continue
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        if src.count(old) != 1:
            print(f"{name:40} PATCH-FAILED ({src.count(old)} matches in {rel})")
            bad += 1
            continue
        suites = (["a", "b", "threads", "relay", "unit"] if a.suite == "all"
                  else must.split() if a.suite == "must" else [a.suite])
        row = []
        try:
            open(path, "w").write(src.replace(old, new))
            for s in suites:
                if s not in SUITES:
                    row.append("-"); continue
                rc, out = run(SUITES[s])
                if "error[" in out or "could not compile" in out:
                    row.append("BUILD?"); bad += 1; continue
                killed = rc != 0
                row.append("KILLED" if killed else "survived")
                if not killed and s in must.split():
                    bad += 1
                    row[-1] = "SURVIVED"
        finally:
            open(path, "w").write(src)
        print(f"{name:40} " + " ".join(f"{x:8}" for x in row), flush=True)
    return 1 if bad else 0


sys.exit(main())
