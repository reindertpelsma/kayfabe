#!/usr/bin/env python3
"""Mutation check of crates/kf-chan/tests/relay_fairness.rs (+ the LoopPolicy unit test) against PRODUCTION mutants of the worker loop.
Each mutant is an exact-text replacement; the file is restored after the run. KILLED = at least two tests fail.
Usage: scripts/fair_mutation_check.py [mutant ...]   (repository root; CARGO_TARGET_DIR honoured)"""
import re
import subprocess
import sys

TEST = ["cargo", "test", "--release", "-p", "kf-chan", "--test", "relay_fairness", "--lib", "-j2", "--no-fail-fast"]
W = "crates/kf-chan/src/worker.rs"
MUTANTS = [
    ("look_every_effectively_never", W, "pub const LOOK_EVERY: u32 = 4;", "pub const LOOK_EVERY: u32 = 1_000_000;"),
    ("look_arm_drops_the_event_peek", W,
     "                        if event {\n                            RelayStats::bump(&relay_stats.wake_event);\n                            let _ = relay.peek(Peek::Event);\n                        }\n                    }\n                    if relay.waiting() && relay.peek(Peek::Spin) {\n                        RelayStats::bump(&relay_stats.wake_spin);\n                    }\n                    continue;",
     "                        if event {\n                            RelayStats::bump(&relay_stats.wake_event);\n                        }\n                    }\n                    if relay.waiting() && relay.peek(Peek::Spin) {\n                        RelayStats::bump(&relay_stats.wake_spin);\n                    }\n                    continue;"),
    ("look_arm_drops_the_spin_peek", W,
     "                    if relay.waiting() && relay.peek(Peek::Spin) {\n                        RelayStats::bump(&relay_stats.wake_spin);\n                    }\n                    continue;",
     "                    if relay.waiting() && false {\n                        RelayStats::bump(&relay_stats.wake_spin);\n                    }\n                    continue;"),
    ("look_arm_does_not_dispatch", W,
     "                        let (doorbell, event) = dispatch(&ready);\n                        if doorbell {\n                            RelayStats::bump(&relay_stats.wake_doorbell);\n                        }\n                        if event {\n                            RelayStats::bump(&relay_stats.wake_event);\n                            let _ = relay.peek(Peek::Event);\n                        }\n                    }\n                    if relay.waiting() && relay.peek(Peek::Spin) {",
     "                        let (doorbell, event) = (false, false);\n                        if doorbell {\n                            RelayStats::bump(&relay_stats.wake_doorbell);\n                        }\n                        if event {\n                            RelayStats::bump(&relay_stats.wake_event);\n                            let _ = relay.peek(Peek::Event);\n                        }\n                    }\n                    if relay.waiting() && relay.peek(Peek::Spin) {"),
    ("busy_pass_never_asks_for_a_look", W, "            if self.busy_passes >= LOOK_EVERY {", "            if false {"),
]


def run():
    p = subprocess.run(TEST, capture_output=True, text=True)
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", p.stdout, re.M)))
    ok = len(re.findall(r"^test \S+ \.\.\. ok", p.stdout, re.M))
    return ("could not compile" not in p.stderr and "error[" not in p.stderr), ok, failed


want = set(sys.argv[1:])
built, ok, failed = run()
print(f"UNMUTATED: built={built} ok={ok} failed={failed}", flush=True)
if not built or failed:
    sys.exit(2)
bad = 0
for name, path, old, new in MUTANTS:
    if want and name not in want:
        continue
    src = open(path).read()
    if src.count(old) != 1:
        print(f"{name}: PATCH DOES NOT APPLY ({src.count(old)})")
        bad += 1
        continue
    open(path, "w").write(src.replace(old, new))
    try:
        built, ok, failed = run()
    finally:
        open(path, "w").write(src)
    status = "KILLED" if built and len(failed) >= 2 else ("ONE KILL" if built and failed else ("DOES NOT BUILD" if not built else "SURVIVED"))
    bad += status != "KILLED"
    print(f"{name}: {status} by {len(failed)}: {failed}", flush=True)
sys.exit(1 if bad else 0)
