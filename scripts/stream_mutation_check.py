#!/usr/bin/env python3
"""GPU-free source mutants for the Phase 2 streamed classification of oversized USER segments.

Usage: python3 -B scripts/stream_mutation_check.py OUTDIR [MUTANT ...]   (CARGO_TARGET_DIR is honoured)

Follows scripts/descriptor_mutation_check.py: a green baseline first, exact one-site source edits, the file
restored in a finally block, a compile error never counts as a kill. Each mutant must fail at least two named
tests. The fourteen mutants are those of traces/descriptor_bound_20261018/phase2.md (2026-10-22, PR #79), redefined
here as committed edits because that campaign was run by hand. The review of PR #360 added R1, R2, R5, R7, R8, R9, R11
and the two S1 mutants. R10 (the pump's `aux.receipts.clear()` after `switch_source`, userpump.rs `migrate`) is EQUIVALENT and
has no mutant: `switch_source` calls `cancel_stream`, which sets `discard_receipts`, and the next turn's `Obs` phase drops
the receipts (`begin_stream` also clears them), so the explicit clear changes no observable state.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CH = "crates/kf-chan/src/"
QR = "crates/kf-qemu/src/userread.rs"
PUMP = "crates/kf-qemu/src/userpump.rs"
CHAN_TESTS = ["cargo", "test", "-p", "kf-chan", "--lib", "--test", "user_turn_unit", "--test", "ring",
              "--no-fail-fast"]
QEMU_TESTS = ["cargo", "test", "-p", "kf-qemu", "--lib", "userread", "--no-fail-fast"]

# name: (file, old, new, test command)
M = {
    "part_yield_removed": (CH + "user_turn.rs",
        "                self.again = true;\n                self.set_need(Need::Again);\n                self.idle_step(false)\n            }\n            Ok(Fed::Done)",
        "                self.again = true;\n                self.set_need(Need::Again);\n                Ok(Step::More)\n            }\n            Ok(Fed::Done)", CHAN_TESTS),
    "packet_state_forgotten": (CH + "user_scan.rs",
        "        *self = next;\n        *cursor = position;\n        Ok(outcome)",
        "        next.pending = None;\n        *self = next;\n        *cursor = position;\n        Ok(outcome)", CHAN_TESTS),
    "end_does_not_stop_parts": (CH + "user_chunk.rs",
        "PartOutcome::Ended { .. } => self.ended = true,", "PartOutcome::Ended { .. } => {}", CHAN_TESTS),
    "trigger_forwarded": (CH + "user_chunk.rs",
        "PartOutcome::Trigger(t) => return Err(Refusal::StagedSpanBudget { at_word: t.start }),",
        "PartOutcome::Trigger(_) => {}", CHAN_TESTS),
    "only_last_receipt_checked": (QR,
        "    for receipt in receipts {\n        let r = &receipt.0;\n        match resolve(",
        "    for receipt in receipts.iter().rev().take(1) {\n        let r = &receipt.0;\n        match resolve(", QEMU_TESTS),
    "scan_committed_per_part": (CH + "user_turn.rs",
        "            Ok(Fed::More) => {\n                self.again = true;",
        "            Ok(Fed::More) => {\n                if let Some(s) = self.stream.as_ref() {\n                    self.scan = s.clone().into_scan();\n                }\n                self.again = true;", CHAN_TESTS),
    "stream_survives_retry": (CH + "user_turn.rs",
        "        let restart = self.cancel_stream();", "        let restart = false;", CHAN_TESTS),
    "receipt_topology_not_checked": (QR,
        "    if first.generation == 0 || first.ram.generation.load(Ordering::Relaxed) != first.generation {",
        "    if first.generation == 0 {", QEMU_TESTS),
    "receipt_owners_not_checked": (QR,
        "        if !Weak::ptr_eq(&r.rows, &first.rows)\n            || !std::ptr::eq(r.ram, first.ram)\n            || r.generation != first.generation\n        {",
        "        if false {", QEMU_TESTS),
    "receipt_count_unbounded": (QR,
        "    if receipts.is_empty() || receipts.len() > usize::from(kf_chan::user_chunk::MAX_PARTS) {",
        "    if receipts.is_empty() {", QEMU_TESTS),
    "original_word_changed": (CH + "user_turn.rs",
        "        let raw = stream.entry();\n        io.publish_streamed(",
        "        let raw = stream.entry() ^ (1 << 41);\n        io.publish_streamed(", CHAN_TESTS),
    "staged_budget_counter_dropped": (CH + "user_turn.rs",
        "                self.life.staged_span_budget_refused();\n", "", CHAN_TESTS),
    "one_copy_exceeds_snapshot_bound": (CH + "user_chunk.rs",
        "Some((self.va.checked_add(self.next)?, left.min(self.maximum)))",
        "Some((self.va.checked_add(self.next)?, left.min(self.maximum.saturating_add(4))))", CHAN_TESTS),
    # ---- the PR #360 review's own mutants (R1..R11); R10 is documented as equivalent below
    "r1_per_copy_bound_above_staging_accepted": (CH + "user_chunk.rs",
        "            || maximum > crate::user_storage::MAX_SOURCE_BYTES\n", "", CHAN_TESTS),
    "r2_part_ceiling_plus_one": (CH + "user_chunk.rs",
        "        if parts > u64::from(MAX_PARTS) {", "        if parts > u64::from(MAX_PARTS) + 1 {", CHAN_TESTS),
    "r5_switch_source_keeps_the_stream": (CH + "user_turn.rs",
        "        self.cancel_stream();\n        self.generation = generation;", "        self.generation = generation;", CHAN_TESTS),
    "r7_retire_keeps_the_stream": (CH + "user_turn.rs",
        "        self.cancel_stream();\n        self.lazy.invalidate(Invalidate::Retire);", "        self.lazy.invalidate(Invalidate::Retire);", CHAN_TESTS),
    "r8_part_word_count_not_checked": (CH + "user_chunk.rs",
        "            != Some(bytes)\n        {\n            return Err(Refusal::Bounds);", "            == Some(bytes)\n        {\n            return Err(Refusal::Bounds);", CHAN_TESTS),
    "r9_receipt_piece_bound_dropped": (QR,
        "            || r.pieces > PIECES\n", "", QEMU_TESTS),
    "r11_restart_does_not_reserve": (PUMP,
        "        Err(Failure::Retry) if restart_owed => Some(Need::Again),\n", "", QEMU_TESTS),
    "s1_busy_publication_restarts_the_stream": (CH + "user_turn.rs",
        "                    Phase::PubStream => io.retry_keeps_stream(),", "                    Phase::PubStream => false,", CHAN_TESTS),
    "s1_busy_part_copy_restarts_the_stream": (CH + "user_turn.rs",
        "                    Phase::Part => true,", "                    Phase::Part => false,", CHAN_TESTS),
    "guest_cursor_does_not_wrap": (CH + "user_turn.rs",
        "self.next_get = self.cursor.checked_add(1).ok_or(\"USER_RING_CURSOR\")? & (entries - 1);",
        "self.next_get = self.cursor.checked_add(1).ok_or(\"USER_RING_CURSOR\")?;", CHAN_TESTS),
}


def run(cmd, log):
    with log.open("w") as f:
        p = subprocess.run(cmd, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, timeout=1800)
    return p.returncode, log.read_text(errors="replace")


def failed(text):
    return sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED$", text, re.M)))


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    names = sys.argv[2:] or list(M)
    for tests in (CHAN_TESTS, QEMU_TESTS):
        if any(M[n][3] is tests for n in names):
            rc, text = run(tests, out / f"baseline_{tests[3]}.log")
            if rc != 0:
                sys.exit(f"baseline for {tests[3]} is not green: {failed(text)}")
    result = {}
    for name in names:
        path, old, new, tests = M[name]
        file = ROOT / path
        original = file.read_text()
        if original.count(old) != 1:
            sys.exit(f"{name}: the edit site occurs {original.count(old)} times in {path}")
        try:
            file.write_text(original.replace(old, new))
            rc, text = run(tests, out / f"{name}.log")
        finally:
            file.write_text(original)
        killers = failed(text)
        compiled = "could not compile" not in text and "error[E" not in text
        result[name] = {"compiled": compiled, "killers": killers, "ok": compiled and len(killers) >= 2}
        print(name, "OK" if result[name]["ok"] else "FAIL", len(killers), flush=True)
    (out / "result.json").write_text(json.dumps(result, indent=1))
    sys.exit(0 if all(r["ok"] for r in result.values()) else 1)


main()
