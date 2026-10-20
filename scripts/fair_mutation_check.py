#!/usr/bin/env python3
"""Mutation check of crates/kf-chan/tests/relay_fairness.rs (+ the LoopPolicy unit test) against PRODUCTION mutants of the worker loop.
Each mutant is an exact-text replacement; the file is restored after the run. KILLED = at least two tests fail.
Usage: scripts/fair_mutation_check.py [mutant ...]   (repository root; CARGO_TARGET_DIR honoured)"""
import re
import subprocess
import sys

TEST = ["cargo", "test", "--release", "-p", "kf-chan", "--features", "chaos-hooks", "--test", "relay_fairness", "--test", "wake_registry", "--lib", "-j2", "--no-fail-fast"]
W = "crates/kf-chan/src/worker.rs"
# 2026-10-20 (simple relay): the LOOK_EVERY / Look-arm mutants pinned deleted code; these are the mutants of the one loop.
MUTANTS = [
    ("no_level_evaluation", W, "let rang = if after == AfterPass::Continue {", "let rang = if true {"),
    ("evaluation_only_when_idle", W, "let rang = if after == AfterPass::Continue {", "let rang = if after != AfterPass::Idle {"),
    ("busy_pass_never_polls", W, "let spin = n > 0 || rang || relay.unsourced();", "let spin = n > 0 || rang || relay.unsourced();\n        if n > 0 { continue; }"),
    ("block_despite_unsourced", W, "let spin = n > 0 || rang || relay.unsourced();", "let spin = n > 0 || rang;"),
    ("dispatch_skipped", W, "            let (doorbell, event) = dispatch(&ready);", "            let (doorbell, event) = (false, false);"),
    ("policy_never_looks", W, "            if self.look {\n                AfterPass::Look", "            if false {\n                AfterPass::Look"),
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
