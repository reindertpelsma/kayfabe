#!/usr/bin/env python3
"""Mutation check of crates/kf-chan/tests/user_loop_model_v2.rs against PRODUCTION source mutants of kf-chan.

Each mutant is an exact-text replacement in one source file; the file is restored after the run. The unmutated
suite must be green first. A mutant is KILLED by every test of user_loop_model_v2 that fails; the requirement is >= 2.
Usage: scripts/v2_mutation_check.py [mutant-name ...]  (run from the repository root; CARGO_TARGET_DIR is honoured)
"""
import re
import subprocess
import sys

TEST = ["cargo", "test", "--release", "-p", "kf-chan", "--test", "user_loop_model_v2", "-j2", "--", "--test-threads=4", "--skip", "p7_two_clients", "--skip", "p7_a_dependency", "--skip", "p8_the_same_with_the_record", "--skip", "p6_a_lapped"]
MUTANTS = [
    ("put_cache_idles_on_a_cached_read", "crates/kf-chan/src/user_lazy.rs",
     "Some((g, put)) if g == generation && put != cursor => Some(put),",
     "Some((g, put)) if g == generation => Some(put),"),
    ("b_skips_while_a_wait_is_pending", "crates/kf-chan/src/user_lazy.rs",
     "self.wait_pending || sync_waiting,",
     "sync_waiting,"),
    ("look_needed_ignores_demand", "crates/kf-chan/src/user_loop.rs",
     "demand || waiting\n}",
     "waiting\n}"),
    ("marker_wait_satisfied_by_pops", "crates/kf-chan/src/user_loop.rs",
     "Self::Marker { target } => marks >= target,",
     "Self::Marker { target } => popped >= target,"),
    ("end_batch_forgets_work_in_flight", "crates/kf-chan/src/user_turn.rs",
     "if self.fetch.fetched() < self.fetch.issued() {\n            // published work in flight",
     "if false {\n            // published work in flight"),
    ("wait_never_latched", "crates/kf-chan/src/user_lazy.rs",
     "pub fn wait_set(&mut self) {\n        self.wait_pending = true;",
     "pub fn wait_set(&mut self) {\n        self.wait_pending = false;"),
    ("popped_wait_target_off_by_one_high", "crates/kf-chan/src/user_loop.rs",
     "Self::Popped { target } => popped >= target,",
     "Self::Popped { target } => popped > target,"),
    ("contended_wait_never_satisfied", "crates/kf-chan/src/user_loop.rs",
     "Self::Contended => true,",
     "Self::Contended => false,"),
]


def run():
    p = subprocess.run(TEST, capture_output=True, text=True)
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", p.stdout, re.M)))
    ok = len(re.findall(r"^test \S+ \.\.\. ok", p.stdout, re.M))
    built = "could not compile" not in p.stderr and "error[" not in p.stderr
    return built, ok, failed, p


FALLBACK = ["cargo", "test", "--release", "-p", "kf-chan", "--test", "user_loop_model", "--test", "user_lazy_turn",
            "--test", "user_turn_unit", "--test", "user_lazy_model", "-j2", "--no-fail-fast"]
# redundant-defence / unmodelled-wait mutants: v2 checks liveness, not staleness latency, so they may survive v2; the
# older suites must kill them
EXPECTED_SURVIVORS = {
    "b_skips_while_a_wait_is_pending",
    "wait_never_latched",
    "end_batch_forgets_work_in_flight",
    "contended_wait_never_satisfied",
}


def fallback():
    p = subprocess.run(FALLBACK, capture_output=True, text=True)
    return sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", p.stdout, re.M)))


def main():
    want = set(sys.argv[1:])
    built, ok, failed, p = run()
    print(f"UNMUTATED: built={built} ok={ok} failed={failed}", flush=True)
    if not built or failed:
        sys.exit(2)
    bad = 0
    for name, path, old, new in MUTANTS:
        if want and name not in want:
            continue
        src = open(path).read()
        if src.count(old) != 1:
            print(f"{name}: PATCH DOES NOT APPLY exactly once ({src.count(old)})")
            bad += 1
            continue
        open(path, "w").write(src.replace(old, new))
        try:
            built, ok, failed, p = run()
        finally:
            open(path, "w").write(src)
        status = "KILLED" if built and len(failed) >= 2 else ("ONE KILL" if built and failed else ("DOES NOT BUILD" if not built else "SURVIVED"))
        extra = ""
        if status != "KILLED" and name in EXPECTED_SURVIVORS:
            open(path, "w").write(src.replace(old, new))
            try:
                other = fallback()
            finally:
                open(path, "w").write(src)
            extra = f" | older suites kill it: {len(other)} {other[:3]}"
            if len(other) >= 2:
                status = "SURVIVES v2 (expected), KILLED by older suites"
        if not status.startswith("KILLED") and not status.startswith("SURVIVES v2"):
            bad += 1
        print(f"{name}: {status} by {len(failed)}: {failed}{extra}", flush=True)
    sys.exit(1 if bad else 0)


main()
