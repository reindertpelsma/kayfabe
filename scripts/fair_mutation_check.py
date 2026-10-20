#!/usr/bin/env python3
"""Mutation check of the lock-holder handoff + sleep-safety design (kf-chan relay_fairness, handoff_protocol,
user_loop_model_v3_pointer, lib tests; kf-qemu lib tests for the pt-lock / userpump mutants).
Each mutant is an exact-text replacement of PRODUCTION code; the file is restored after the run. KILLED = at least two tests fail.
Usage: scripts/fair_mutation_check.py [mutant ...]   (repository root; CARGO_TARGET_DIR honoured)"""
import re
import subprocess
import sys

CHAN = ["cargo", "test", "--release", "-p", "kf-chan", "--lib", "--test", "relay_fairness", "--test", "handoff_protocol",
        "--test", "user_loop_model_v3_pointer", "-j2", "--no-fail-fast"]
QEMU = ["cargo", "test", "--release", "-p", "kf-qemu", "--lib", "-j2", "--no-fail-fast"]
# the source pins that live in kf-chan's model tests guard the kf-qemu relay code too
PINS = ["cargo", "test", "--release", "-p", "kf-chan", "--test", "user_loop_model_v2", "--test", "relay_fairness", "-j2", "--no-fail-fast", "pin_"]
H = "crates/kf-chan/src/handoff.rs"
U = "crates/kf-chan/src/user_loop.rs"
W = "crates/kf-chan/src/worker.rs"
P = "crates/kf-qemu/src/ptlock.rs"
UP = "crates/kf-qemu/src/userpump.rs"
QEMU_PINS = [QEMU, PINS]
MUTANTS = [
    ("handoff_changed_since_never", CHAN, H, "self.epoch.load(Ordering::SeqCst) != seen", "{ let _ = seen; false }"),
    ("handoff_bump_is_a_load", CHAN, H, "self.epoch.fetch_add(1, Ordering::SeqCst);", "let _ = self.epoch.load(Ordering::SeqCst);"),
    ("handoff_released_never_rings", CHAN, H, "                ring(who);\n", ""),
    ("handoff_lost_never_retries", CHAN, H, "        let again = self.changed_since(seen);", "        let again = { let _ = seen; self.changed_since(self.epoch.load(Ordering::SeqCst)) };"),
    ("cover_popped_claims_event_source", CHAN, U,
     "            change: \"the private GP_GET passes the target (pops raise nothing)\",\n            source: None,\n            can_miss: true,",
     "            change: \"the private GP_GET passes the target (pops raise nothing)\",\n            source: Some(WakeSource::EngineEvent),\n            can_miss: false,"),
    # NOTE: `c.source.is_some() && !c.can_miss` -> `c.source.is_some()` is an EQUIVALENT mutant today (no row has a source that can miss).
    ("may_block_always", CHAN, U, "c.source.is_some() && !c.can_miss", "true"),
    ("idle_never_spins", CHAN, U, "if unannounced { Idle::Spin } else { Idle::Park }", "{ let _ = unannounced; Idle::Park }"),
    ("look_needed_on_contention", CHAN, U, "pub const fn look_needed(_why: Peek, _demand: bool, waiting: bool) -> bool {\n    waiting", "pub const fn look_needed(_why: Peek, demand: bool, waiting: bool) -> bool {\n    waiting || demand"),
    ("worker_ignores_unannounced", CHAN, W, "if user_loop::idle(relay.unannounced()) == user_loop::Idle::Spin {", "if user_loop::idle(false) == user_loop::Idle::Spin {"),
    ("park_timeout_ignores_infinite", CHAN, W, "    } else if infinite {\n        PollTimeout::Blocking", "    } else if infinite && false {\n        PollTimeout::Blocking"),
    ("ptlock_tells_before_release", QEMU_PINS, P, "        drop(self.g.take());\n        self.hub.released();", "        self.hub.released();\n        drop(self.g.take());"),
    ("ptlock_never_tells", QEMU_PINS, P, "        self.hub.released();\n    }\n}\n\n#[cfg(test)]", "    }\n}\n\n#[cfg(test)]"),
    ("userpump_loser_does_not_register", QEMU_PINS, UP, "if self.handoff.lost(record.idx, e0) {", "if { let _ = e0; false } {"),
    ("userpump_arm_loser_does_not_register", QEMU_PINS, UP, "if self.handoff.lost(idx, e0) {", "if { let _ = e0; false } {"),
]


def run(cmd):
    cmds = cmd if isinstance(cmd[0], list) else [cmd]
    built, ok, failed = True, 0, set()
    for c in cmds:
        p = subprocess.run(c, capture_output=True, text=True)
        failed |= set(re.findall(r"^test (\S+) \.\.\. FAILED", p.stdout, re.M))
        ok += len(re.findall(r"^test \S+ \.\.\. ok", p.stdout, re.M))
        built &= "could not compile" not in p.stderr and "error[" not in p.stderr
    return built, ok, sorted(failed)


want = set(sys.argv[1:])
bad = 0
base = {}
for cmd in (CHAN, QEMU_PINS):
    if want and not any(m[0] in want and m[1] is cmd for m in MUTANTS):
        continue
    built, ok, failed = run(cmd)
    print(f"UNMUTATED {'kf-chan' if cmd is CHAN else 'kf-qemu+pins'}: built={built} ok={ok} failed={failed}", flush=True)
    if not built or failed:
        sys.exit(2)
for name, cmd, path, old, new in MUTANTS:
    if want and name not in want:
        continue
    src = open(path).read()
    if src.count(old) != 1:
        print(f"{name}: PATCH DOES NOT APPLY ({src.count(old)})")
        bad += 1
        continue
    open(path, "w").write(src.replace(old, new))
    try:
        built, ok, failed = run(cmd)
    finally:
        open(path, "w").write(src)
    status = "KILLED" if built and len(failed) >= 2 else ("ONE KILL" if built and failed else ("DOES NOT BUILD" if not built else "SURVIVED"))
    bad += status != "KILLED"
    print(f"{name}: {status} by {len(failed)}: {failed}", flush=True)
sys.exit(1 if bad else 0)
