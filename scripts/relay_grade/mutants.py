#!/usr/bin/env python3
"""G10 mutation check of the grader (docs/design/V3_RELAY_GRADER_20261020.md, section G10).

Each mutant is one source edit (production code, or the adapter's mirror of the kf-qemu glue, which cannot be built GPU-free).
For each: apply, build and run the grade_* targets, count the DISTINCT tests that go red, restore. A mutant is KILLED when >= 2
distinct tests fail (owner's bar). Usage:

    CARGO_TARGET_DIR=... python3 -I scripts/relay_grade/mutants.py [name ...]   # default: all
    GRADE_MUTANT_TESTS="--test grade_model --test grade_locks"  narrows the targets (default: all four grade_* targets)

Prints one line per mutant: KILLED/SURVIVED, the count and the names. Exit 1 when any mutant survives. Always restores the files
(also on Ctrl-C).
"""
import os, re, subprocess, sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

MUTANTS = [
    # (name, file, old, new, what it breaks)
    ("wake_not_registered", "crates/kf-chan/src/worker.rs",
     "let spin = n > 0 || rang || relay.unsourced();", "let spin = n > 0 || rang;",
     "the worker blocks with an unsourced obligation (the retry set, a pop wait)"),
    ("look_skipped_on_busy_pass", "crates/kf-chan/src/worker.rs",
     "fn default() -> Self {\n        Self { look: true }", "fn default() -> Self {\n        Self { look: false }",
     "a busy worker never looks (F3)"),
    ("demand_not_rerung", "crates/kf-chan/src/user_loop.rs",
     "    if i.demand {\n        Look::Ring {\n            clear_wait: false,\n            pops_only: false,\n        }\n    } else {\n        Look::Leave\n    }\n}",
     "    let _ = i.demand;\n    Look::Leave\n}",
     "demand left by a lost lock is not re-rung (the budget's end state, F1)"),
    ("event_consumed_without_ring", "crates/kf-chan/src/user_loop.rs",
     "        return if i.demand || i.wait.is_some() {", "        return if i.demand {",
     "a look that finds the record busy drops the waiting relay (F5, seed 20)"),
    ("marker_not_repeeked", "crates/kf-chan/src/user_loop.rs",
     "        if w.satisfied(marks, popped) {", "        if w.satisfied(marks, popped) && false {",
     "a satisfied wait is not rung by the level evaluation"),
    ("claim_not_released", "crates/kf-trap/src/token.rs",
     "State::Busy => (State::Idle, Release::Idled),", "State::Busy => (State::Busy, Release::Idled),",
     "a served token stays BUSY: later doorbells are absorbed"),
    ("again_flag_lost", "crates/kf-trap/src/token.rs",
     "State::Busy => State::BusyRung,", "State::Busy => State::Busy,",
     "a ring while BUSY is dropped (no ActAgain / republish)"),
    ("park_ignores_sequence", "crates/kf-trap/src/wake.rs",
     "if (cur >> POLLER_BITS) != seen {", "if (cur >> POLLER_BITS) != seen && false {",
     "the park ignores work that arrived during the scan (lost wakeup)"),
    ("registration_after_holder_check", "crates/kf-chan/tests/grade_adapter.rs",
     "    pub fn contended_leaves_demand() -> bool {\n        true\n    }", "    pub fn contended_leaves_demand() -> bool {\n        false\n    }",
     "MIRROR: the contended arm does not re-register demand (userpump `record.request()` removed)"),
    ("unsourced_ignores_carried", "crates/kf-chan/tests/grade_adapter.rs",
     "        demand || (waiting && carried)\n", "        let _ = (waiting, carried);\n        demand\n",
     "MIRROR: a pop wait (no interrupt) does not keep the loop spinning"),
    ("budget_reintroduced", "crates/kf-chan/src/user_loop.rs",
     "pub const fn look(i: LookIn) -> Look {\n", "pub const fn look(i: LookIn) -> Look {\n    if i.demand && i.record_busy && i.wait.is_none() {\n        return Look::Leave;\n    }\n",
     "a look gives up on demand behind a busy record (an exhausted-budget stand-in)"),
]


def run(names):
    tests = os.environ.get("GRADE_MUTANT_TESTS", "--test grade_model --test grade_locks --test grade_static --test grade_stress").split()
    survivors = []

    def red_of():
        p = subprocess.run(["nice", "-n", "19", "cargo", "test", "--release", "-p", "kf-chan", "-j2", *tests, "--no-fail-fast",
                            "--", "--test-threads=2"], cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
        red = set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M))
        if re.search(r"^error(\[E\d+\])?:", out, re.M):
            red.add("<compile error>")
        return red

    base = red_of()
    print(f"baseline red (excluded from every count): {sorted(base)}", flush=True)
    for name, rel, old, new, what in MUTANTS:
        if names and name not in names:
            continue
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        if src.count(old) != 1:
            print(f"{name}: NOT APPLICABLE (anchor found {src.count(old)} times in {rel})", flush=True)
            continue
        try:
            open(path, "w").write(src.replace(old, new))
            red = sorted(red_of() - base)
        finally:
            open(path, "w").write(src)
        verdict = "KILLED" if len(red) >= 2 else "SURVIVED"
        if verdict == "SURVIVED":
            survivors.append(name)
        print(f"{name}: {verdict} by {len(red)} ({what}): {', '.join(red[:12])}", flush=True)
    return survivors


if __name__ == "__main__":
    s = run(set(sys.argv[1:]))
    sys.exit(1 if s else 0)
