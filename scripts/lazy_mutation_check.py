#!/usr/bin/env python3
"""Mutation check of lazy observe in the Windows USER-ring relay (docs/design/V3_USER_RING_LIFECYCLE_20261016.md,
"Lazy observe (production)").

Each mutant is an exact-text patch of PRODUCTION source (the rules live in `kf_chan::user_lazy` / `user_turn`, and the
pump wiring in `kf-qemu/src/userpump.rs`). After each patch the GPU-free lazy suites are run and the tests that FAIL
are listed. A mutant that no test kills is a missing test; one killed by a single test is listed as thin. Compile errors do
not count as kills. Sources are restored after every mutant and at exit.

usage: scripts/lazy_mutation_check.py OUTDIR [name ...]
       CARGO_TARGET_DIR keeps the build elsewhere. GPU-free; never touches hardware.
Run on an idle machine: a full pass is about an hour (kf-qemu mutants compile the pump).
"""
import hashlib
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CH = "crates/kf-chan/src/"
QP = "crates/kf-qemu/src/userpump.rs"

# name -> (description, [(file, old, new)], runs the kf-qemu pins too)
M = {}


def mutant(name, desc, edits, qemu=False):
    M[name] = (desc, edits, qemu)


mutant(
    "no_invalidate_on_mode_change",
    "the cache is not dropped when the lifecycle mode changes",
    [(CH + "user_lazy.rs", "                self.invalidate(Invalidate::ModeChange);\n", "")],
)
mutant(
    "cache_survives_migration",
    "the cache survives a ring migration (no invalidation, and the generation key is ignored)",
    [
        (CH + "user_turn.rs", "        self.lazy.invalidate(Invalidate::Migration);\n", ""),
        (CH + "user_lazy.rs", "Some((g, put)) if g == generation && put != cursor => Some(put),", "Some((_, put)) if put != cursor => Some(put),"),
    ],
)
mutant(
    "idle_on_a_cached_read",
    "a cached PUT is used even when the cursor has reached it (idle / end_batch on a cached value)",
    [(CH + "user_lazy.rs", "Some((g, put)) if g == generation && put != cursor => Some(put),", "Some((g, put)) if g == generation => Some(put),")],
)
mutant(
    "stale_credits_over_counted",
    "Fetch::available counts one credit too many",
    [(CH + "user_scan.rs", "self.entries - 1 - self.pending.len() as u32", "self.entries - self.pending.len() as u32")],
)
mutant(
    "low_credit_never_forces_a_look",
    "the credit floor never forces an observation (the stale count is trusted at any level)",
    [(CH + "user_lazy.rs", "            available,\n            segment_open,\n            self.ring_full,", "            u32::MAX,\n            segment_open,\n            self.ring_full,")],
)
mutant(
    "stale_reference_trusted",
    "a SYNC_WAIT gate is released one marker early (a stale REFERENCE would be trusted as progress)",
    [(CH + "user_gate.rs", "Some(w) if processed >= w =>", "Some(w) if processed + 1 >= w =>")],
)
mutant(
    "unissued_reference_trusted",
    "Marks::peek believes a REFERENCE that names markers not issued yet",
    [(CH + "user_gate.rs", "if delta <= self.issued - self.seen {", "if delta <= self.issued - self.seen + 8 {")],
)
mutant(
    "b_skips_while_a_wait_is_pending",
    "a pending wait / draining gate no longer forces the look",
    [(CH + "user_lazy.rs", "            self.wait_pending || sync_waiting,\n", "            false,\n")],
)
mutant(
    "wait_never_latched",
    "the relay does not remember that it ended a turn waiting",
    [(CH + "user_turn.rs", "            self.lazy.wait_set();\n", "")],
)
mutant(
    "open_segment_does_not_force",
    "an open segment no longer forces the look",
    [(CH + "user_lazy.rs", "            available,\n            segment_open,\n            self.ring_full,", "            available,\n            false,\n            self.ring_full,")],
)
mutant(
    "interval_never_fires",
    "the periodic look never fires",
    [(CH + "user_lazy.rs", "    } else if turn_no.is_multiple_of(interval) {", "    } else if false && turn_no.is_multiple_of(interval) {")],
)
mutant(
    "interval_not_capped_by_the_guest_ring",
    "the interval is not capped at half the guest ring",
    [(CH + "user_lazy.rs", "    interval.min(u64::from(src_entries / 2).max(1))", "    interval")],
)
mutant(
    "no_ring_full_guard",
    "a skipped turn that saw the guest ring full does not force the next look (D6)",
    [(CH + "user_lazy.rs", "        if self.skipped {\n            self.ring_full = true;\n        }", "")],
)
mutant(
    "window_uses_the_cache",
    "rule A applies in every mode (a Window / Scan turn may use or fill the cache)",
    [(CH + "user_lazy.rs", "        self.cfg.put_cache && mode == Mode::Pointer\n", "        self.cfg.put_cache\n")],
)
mutant(
    "b_applies_in_every_mode",
    "rule B applies in every mode (a Scan / Window turn may skip its look)",
    [(CH + "user_lazy.rs", "        self.cfg.interval > 1 && mode == Mode::Pointer\n", "        self.cfg.interval > 1\n")],
)
mutant(
    "observed_keeps_the_wait_flag",
    "an observation does not clear the wait latch (every later turn looks)",
    [(CH + "user_lazy.rs", "        self.wait_pending = false;\n        self.ring_full = false;\n", "        self.ring_full = false;\n")],
)
mutant(
    "walk_gate_after_the_cache_is_dropped",
    "the relay no longer asks Io::walk_busy before reading",
    [(CH + "user_turn.rs", "        if io.walk_busy() {\n            return self.idle_step(false);\n        }\n", "")],
)
mutant(
    "refusal_keeps_the_cache",
    "a refusal (channel reset) does not drop the cache",
    [(CH + "user_turn.rs", "                self.lazy.invalidate(Invalidate::Reset);\n", "")],
)
mutant(
    "retire_keeps_the_cache",
    "retire does not drop the cache",
    [(CH + "user_turn.rs", "        self.lazy.invalidate(Invalidate::Retire);\n", "")],
)
mutant(
    "switches_read_per_turn",
    "the environment is read inside the turn",
    [(CH + "user_lazy.rs", "        self.turn_no = self.turn_no.wrapping_add(1);\n        self.counts.turns += 1;", "        let _ = std::env::var(ENV_PUT_CACHE);\n        self.turn_no = self.turn_no.wrapping_add(1);\n        self.counts.turns += 1;")],
    qemu=True,
)
mutant(
    "pump_never_opts_in",
    "the pump never calls with_lazy (production would run eager)",
    [(QP, "            .with_lazy(lazy_config()),\n", "            ,\n")],
    qemu=True,
)
mutant(
    "pump_switch_read_in_the_turn",
    "the pump reads KF3_PUT_CACHE inside turn()",
    [(QP, "        let out = self.turn_body(me, r);\n        self.report_lifecycle_notices();", "        let _ = std::env::var(kf_chan::user_lazy::ENV_PUT_CACHE);\n        let out = self.turn_body(me, r);\n        self.report_lifecycle_notices();")],
    qemu=True,
)
mutant(
    "pump_walk_gate_dropped",
    "the pump's Io::walk_busy never reports a walk",
    [(QP, "        if me.inbox.walk_busy() {\n            me.inbox.wait_walk(r.idx);\n            return true;\n        }\n        false", "        false")],
    qemu=True,
)
mutant(
    "release_does_not_retire",
    "Pump::release no longer calls Relay::retire",
    [(QP, "        self.core.retire();\n", "        self.core.life.teardown();\n")],
    qemu=True,
)
mutant(
    "pump_counters_never_flushed",
    "the per-turn flush of the lazy counters is gone",
    [(QP, "        if self.core.lazy.flush_due() {\n            me.relay_stats.lazy.add(&self.core.lazy.take_delta());\n        }\n", "")],
    qemu=True,
)

# the child run of 33 scenarios takes minutes under a mutant; it runs once, unmutated (the baseline)
CHILD = "lazy_every_other_scenario_of_this_file_also_holds_with_lazy_observe_on"

FAST = [
    "--lib",
    "--test", "user_lazy_turn",
    "--test", "user_lazy_model",
    "--test", "user_lifecycle_props",
    "--test", "user_lifecycle_stress",
    "--test", "user_turn_unit",
    "--test", "user_gate_rules",
]


TIMEOUT = int(os.environ.get("LAZY_MUT_TIMEOUT", "900"))


def cargo(args, log):
    """Run the tests; a mutant that makes a test HANG is killed too (the whole process group is killed after TIMEOUT
    seconds and the tests still running are listed as `TIMEOUT:<name>`)."""
    import signal

    env = {**os.environ, "KF3_STRESS_SECS": os.environ.get("KF3_STRESS_SECS", "1")}
    timed_out = False
    with open(log, "w") as sink:
        p = subprocess.Popen(["nice", "-n", "19", "cargo", "test", "--release", "-j2", "--no-fail-fast"] + args,
                             cwd=ROOT, stdout=sink, stderr=subprocess.STDOUT, env=env, start_new_session=True)
        try:
            p.wait(timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(p.pid, signal.SIGKILL)
            p.wait()
    text = Path(log).read_text(errors="replace")
    failed = re.findall(r"^test (\S+) \.\.\. FAILED$", text, re.M)
    if timed_out:
        done = set(re.findall(r"^test (\S+) \.\.\. (?:ok|FAILED)$", text, re.M))
        running = [n for n in re.findall(r"^test (\S+) has been running for over", text, re.M) if n not in done]
        failed += [f"TIMEOUT:{n}" for n in sorted(set(running))] or ["TIMEOUT"]
    compile_error = "could not compile" in text
    return (1 if failed else p.returncode), failed, compile_error


def main():
    out = Path(sys.argv[1]).resolve()
    only = sys.argv[2:]
    out.mkdir(parents=True, exist_ok=True)
    names = only or list(M)
    files = {f for n in names for f, _, _ in M[n][1]}
    orig = {f: (ROOT / f).read_bytes() for f in files}
    rows = []
    try:
        # the unmutated run must be green first
        rc, failed, ce = cargo(["-p", "kf-chan"] + FAST, out / "baseline-fast.log")
        if rc != 0:
            print("the unmutated fast suites are not green:", failed)
            return 2
        rc, failed, ce = cargo(["-p", "kf-chan", "--test", "user_lifecycle_model", "--", "lazy_", "--skip", CHILD], out / "baseline-model.log")
        if rc != 0:
            print("the unmutated lazy model tests are not green:", failed)
            return 2
        for n in names:
            desc, edits, qemu = M[n]
            for f, old, new in edits:
                text = (ROOT / f).read_text()
                if text.count(old) != 1:
                    raise SystemExit(f"mutation anchor is not unique in {f} for {n}: {text.count(old)}")
                (ROOT / f).write_text(text.replace(old, new))
            fails, bad = [], False
            for tag, args in [("fast", ["-p", "kf-chan"] + FAST), ("model", ["-p", "kf-chan", "--test", "user_lifecycle_model", "--", "lazy_", "--skip", CHILD])]:
                rc, failed, ce = cargo(args, out / f"{n}-{tag}.log")
                if ce:
                    bad = True
                    break
                fails += failed
            if qemu and not bad:
                rc, failed, ce = cargo(["-p", "kf-qemu", "--lib", "--test", "doorbell_no_put_poll"], out / f"{n}-qemu.log")
                if ce:
                    bad = True
                fails += failed
            for f in files:
                (ROOT / f).write_bytes(orig[f])
            status = "COMPILE ERROR" if bad else ("KILLED" if fails else "SURVIVED")
            rows.append((n, desc, status, sorted(set(fails))))
            print(f"{n}: {status} ({len(set(fails))} failing tests)", flush=True)
    finally:
        for f, b in orig.items():
            (ROOT / f).write_bytes(b)
    lines = ["| mutant | what is broken | result | failing tests |", "| --- | --- | --- | --- |"]
    for n, d, s, f in rows:
        lines.append(f"| `{n}` | {d} | {s} ({len(f)}) | {', '.join(f'`{x}`' for x in f)} |")
    lines.append("")
    lines.append("Restored source SHA256:")
    for f, b in sorted(orig.items()):
        lines.append(f"- `{f}`: `{hashlib.sha256(b).hexdigest()}`")
    (out / "mutation_table.md").write_text("\n".join(lines) + "\n")
    return 0 if all(r[2] == "KILLED" for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
