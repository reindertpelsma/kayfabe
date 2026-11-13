#!/usr/bin/env python3
"""GPU-free source mutants for the 2026-11-13 slow-path instrumentation (SLOW-APPLY, SPLIT-LATE,
USER_RING_SPLIT_WAIT). Usage: python3 -B scripts/slowapply_mutation_check.py OUTDIR [MUTANT ...]

Follows scripts/stream_mutation_check.py: green baseline first, exact one-site edits, the file restored in a
finally block, a compile error never counts as a kill, each mutant must fail at least two named tests.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VA = "crates/kf-mem/src/vasmgr.rs"
UP = "crates/kf-qemu/src/userpump.rs"
VA_TESTS = ["cargo", "test", "-p", "kf-mem", "--lib", "vasmgr", "--no-fail-fast"]
UP_TESTS = ["cargo", "test", "-p", "kf-qemu", "--lib", "split_wait", "--no-fail-fast"]

M = {
    "threshold_inclusive": (VA, "    ns > ms.saturating_mul(1_000_000)", "    ns >= ms.saturating_mul(1_000_000)", VA_TESTS),
    "threshold_unit_us": (VA, "    ns > ms.saturating_mul(1_000_000)", "    ns > ms.saturating_mul(1_000)", VA_TESTS),
    "apply_threshold_600": (VA, "pub const SLOW_APPLY_MS: u64 = 500;", "pub const SLOW_APPLY_MS: u64 = 600;", VA_TESTS),
    "split_threshold_2000": (VA, "pub const SLOW_SPLIT_MS: u64 = 1000;", "pub const SLOW_SPLIT_MS: u64 = 2000;", VA_TESTS),
    "budget_off_by_one": (VA, "        self.seen <= self.cap", "        self.seen < self.cap", VA_TESTS),
    "budget_never_counts": (VA, "        self.seen = self.seen.saturating_add(1);\n        self.seen <= self.cap", "        self.seen <= self.cap", VA_TESTS),
    "apply_max_not_tracked": (VA, "        tm.apply_ns_max = tm.apply_ns_max.max(apply_ns);\n", "", VA_TESTS),
    "slow_apply_not_counted": (VA, "        tm.slow_applies = tm.slow_applies.saturating_add(1);\n", "", VA_TESTS),
    "split_max_not_tracked": (VA, "        tm.split_ns_max = tm.split_ns_max.max(ns);\n", "", VA_TESTS),
    "slow_split_not_counted": (VA, "        tm.slow_splits = tm.slow_splits.saturating_add(1);\n", "", VA_TESTS),
    "split_clock_from_walk_submit": (VA, "self.note_split_done(*ticket, *pdb, *skey, *sat);", "self.note_split_done(*ticket, *pdb, *skey, batch.submitted);", VA_TESTS),
    "mapstat_rows_not_counted": (VA, "        self.scat_rows.fetch_add(rows as u64, o);\n", "", VA_TESTS),
    "mapstat_ram_store_swapped": (VA, "            (&self.ram_calls, &self.ram_ns, &self.ram_ns_max)\n        } else {\n            (&self.store_calls, &self.store_ns, &self.store_ns_max)", "            (&self.store_calls, &self.store_ns, &self.store_ns_max)\n        } else {\n            (&self.ram_calls, &self.ram_ns, &self.ram_ns_max)", VA_TESTS),
    "worst_ignores_split": (VA, "printed_ms != (apply_ns_max / 1_000_000, split_ns_max / 1_000_000)", "printed_ms.0 != apply_ns_max / 1_000_000", VA_TESTS),
    "worst_ns_not_ms": (VA, "printed_ms != (apply_ns_max / 1_000_000, split_ns_max / 1_000_000)", "printed_ms != (apply_ns_max / 1_000, split_ns_max / 1_000)", VA_TESTS),
    "wait_threshold_zero": (UP, "    if !kf_mem::vasmgr::exceeds_ms(ns, kf_mem::vasmgr::SLOW_SPLIT_MS) {", "    if !kf_mem::vasmgr::exceeds_ms(ns, 0) {", UP_TESTS),
    "wait_repeated_per_poll": (UP, "        (false, true) => None,", "        (false, true) => Some(\"waiting\"),", UP_TESTS),
    "wait_end_not_named": (UP, "        (true, _) => Some(\"done\"),", "        (true, _) => None,", UP_TESTS),
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
    for tests in (VA_TESTS, UP_TESTS):
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
