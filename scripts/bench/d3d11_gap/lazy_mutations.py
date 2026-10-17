#!/usr/bin/env python3
"""Mutate a disposable checkout, run the lazy model, and restore every source.

Run while the host is idle, under flock -o /tmp/kayfabe-fastguest.lock.
Usage: lazy_mutations.py CHECKOUT OUTPUT_DIR
Only screened result summaries belong in git; raw cargo output stays on host.
Falsifier of sensitivity: any mutant passes (compile errors do not count).
"""
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys


MUTANTS = [
    ("generation ignored", "user_lazy.rs", "g == generation && put != cursor", "put != cursor"),
    ("idle on cached PUT", "user_lazy.rs", "g == generation && put != cursor", "g == generation"),
    ("explicit invalidation omitted", "user_lazy.rs", "self.last = None;", "// mutation: retained"),
    ("wait observation omitted", "user_lazy.rs", "        || prev_waiting\n", ""),
    ("open-segment observation omitted", "user_lazy.rs", "        || segment_open\n", ""),
    ("low-credit observation omitted", "user_lazy.rs", "        || available < CREDIT_FLOOR\n", ""),
    ("one excess credit", "user_scan.rs", "self.entries - 1 - self.pending.len() as u32", "self.entries - self.pending.len() as u32"),
    ("REFERENCE gate one marker early", "user_gate.rs", "Some(w) if processed >= w", "Some(w) if processed + 1 >= w"),
    ("unissued REFERENCE trusted", "user_gate.rs", "if delta <= self.issued - self.seen {", "if delta <= self.issued - self.seen + 8 {"),
    ("N=1 observation omitted", "user_lazy.rs", "    interval <= 1\n", "    if interval == 1 { return false; }\n    interval <= 1\n"),
]


def main():
    root, out = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    src = root / "crates/kf-chan/src"
    originals = {name: (src / name).read_bytes() for _, name, _, _ in MUTANTS}
    rows = []
    try:
        for i, (name, file, before, after) in enumerate(MUTANTS, 1):
            original = originals[file].decode()
            if original.count(before) != 1:
                raise RuntimeError(f"mutation anchor is not unique: {name}")
            (src / file).write_text(original.replace(before, after))
            log = out / f"mutation-{i:02}.log"
            with log.open("w") as sink:
                run = subprocess.run(
                    ["nice", "-n", "19", "cargo", "test", "--release", "-j2", "-p", "kf-chan", "--test", "user_lazy_model"],
                    cwd=root, stdout=sink, stderr=subprocess.STDOUT, check=False,
                    env={**os.environ, "CARGO_BUILD_JOBS": "2"},
                )
            output = log.read_text()
            failed = re.findall(r"^test (\w+) \.\.\. FAILED$", output, re.M)
            killed = run.returncode != 0 and bool(failed) and "could not compile" not in output
            rows.append((name, "KILLED" if killed else "NOT KILLED", ", ".join(failed) or f"exit={run.returncode}"))
            print(f"{i:02} {name}: {rows[-1][1]} ({len(failed)} failed tests)", flush=True)
            (src / file).write_bytes(originals[file])
    finally:
        for name, original in originals.items():
            (src / name).write_bytes(original)
    summary = ["| Mutation | Result | Failing tests |", "| --- | --- | --- |"]
    summary.extend(f"| {name} | {result} | {failed} |" for name, result, failed in rows)
    summary.append("")
    summary.append("Restored source SHA256:")
    summary.extend(f"- `{name}`: `{hashlib.sha256(original).hexdigest()}`" for name, original in originals.items())
    (out / "mutation_table.md").write_text("\n".join(summary) + "\n")
    if len(rows) != len(MUTANTS) or any(row[1] != "KILLED" for row in rows):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
