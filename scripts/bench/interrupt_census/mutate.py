#!/usr/bin/env python3
"""PF1 host-only counter mutants. Restore source even when a mutant survives."""

import argparse
import json
import re
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "crates/kf-qemu/src/intrdiag.rs"
MUTANTS = {
    "markers": ("bump(&self.markers, index, 1);", "bump(&self.markers, index, 0);"),
    "host_edge_vectors": ("bump(row, vector, 1);", "bump(row, vector, 0);"),
    "guest_raise_vectors": (
        "bump(&self.raises, (vector as usize).min(VECTORS), 1);",
        "bump(&self.raises, (vector as usize).min(VECTORS), 0);",
    ),
    "eventfd_success": ("Metric::Eventfd\n", "Metric::EventfdErrors\n"),
    "isr_cycles": ("self.add(Metric::Cycles, 1);", "self.add(Metric::Cycles, 0);"),
    "isr_duration": (
        "self.add(Metric::CycleNs, stamp.saturating_sub(start));",
        "self.add(Metric::CycleNs, 0);",
    ),
    "merge_candidates": ("self.add(Metric::Merges, 1);", "self.add(Metric::Merges, 0);"),
    "first_tree_write": (
        "self.add(Metric::FirstWrites, 1);",
        "self.add(Metric::FirstWrites, 0);",
    ),
    "tree_writes": (
        "bump(&self.writes, class(reg), 1);",
        "bump(&self.writes, class(reg), 0);",
    ),
    "tree_reads": ("bump(&self.reads, class(reg), 1);", "bump(&self.reads, class(reg), 0);"),
    "stranded_pending": (
        "self.add(Metric::Stranded, 1);",
        "self.add(Metric::Stranded, 0);",
    ),
    "runtime_default": (
        '(setting == Some("1")).then(Self::new)',
        '(setting != Some("0")).then(Self::new)',
    ),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    original = SOURCE.read_text()
    results = []
    try:
        for name, (old, new) in MUTANTS.items():
            assert original.count(old) == 1, (name, "mutation is not unique")
            SOURCE.write_text(original.replace(old, new))
            process = subprocess.run(
                ["nice", "-n", "19", "cargo", "test", "-j2", "--release", "-p", "kf-qemu",
                 "--features", "diag-intr", "--test", "interrupt_census"],
                cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                timeout=300,
            )
            (args.output / (name + ".log")).write_text(process.stdout)
            killed = re.findall(r"^test (\S+) \.\.\. FAILED$", process.stdout, re.MULTILINE)
            row = {"mutant": name, "exit": process.returncode, "killing_tests": killed}
            results.append(row)
            print(json.dumps(row), flush=True)
            assert process.returncode == 101 and len(set(killed)) >= 2, row
            SOURCE.write_text(original)
    finally:
        SOURCE.write_text(original)
        (args.output / "summary.json").write_text(json.dumps(results, indent=2) + "\n")
    restored = subprocess.run(
        ["nice", "-n", "19", "cargo", "test", "-j2", "--release", "-p", "kf-qemu",
         "--features", "diag-intr", "--test", "interrupt_census"],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=300,
    )
    (args.output / "restored-green.log").write_text(restored.stdout)
    assert restored.returncode == 0, "restored observer did not pass"
    print(f"PASS: {len(results)} mutants, >=2 killing tests each; restored source passes")


if __name__ == "__main__":
    main()
