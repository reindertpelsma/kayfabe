#!/usr/bin/env python3
"""Host-only KINDSTAT counter mutants, with source restoration and final green check."""

import json
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[3]
KIND = ROOT / "crates/kf-mem/src/kindstat.rs"
APPLY = ROOT / "crates/kf-mem/src/apply.rs"
STORE = "if let Some(bin) = self.bytes.get(index(raw, host, ram, leaf)) {"
MUTANTS = {
    "raw_kind": (KIND, STORE, STORE.replace("index(raw,", "index(0,")),
    "host_kind": (KIND, STORE, STORE.replace("raw, host,", "raw, 0,")),
    "aperture": (KIND, STORE, STORE.replace("host, ram,", "host, false,")),
    "page_size": (KIND, STORE, STORE.replace("ram, leaf)", "ram, 0)")),
    "bytes": (KIND, "Some(old.saturating_add(bytes))", "Some(old)"),
    "pages4k": (KIND, "self.bytes(raw, host, ram, leaf) / 4096",
                "self.bytes(raw, host, ram, leaf) / 65536"),
    "window": (KIND, "a.saturating_sub(*b)", "*a"),
    "rate": (KIND, "/ self.at_ns as f64", "/ 1.0"),
    "default": (KIND, '(value == Some("1"))', '(value != Some("0"))'),
    "cpu_exclusion": (APPLY, "if net && let Some(diag) = diag",
                      "if let Some(diag) = diag"),
    "held_exclusion": (APPLY, "!failed[i] && placed[i].contains(&(row.va, row.len))",
                       "!failed[i]"),
}


def main():
    output = Path(sys.argv[1])
    output.mkdir(parents=True, exist_ok=True)
    originals = {p: p.read_text() for p in [KIND, APPLY]}
    command = ["nice", "-n", "19", "cargo", "test", "-j2", "--release", "-p", "kf-mem",
               "--features", "diag-kindstat", "--test", "kindstat"]
    results = []
    try:
        for name, (path, old, new) in MUTANTS.items():
            assert originals[path].count(old) == 1, (name, "nonunique mutation")
            path.write_text(originals[path].replace(old, new))
            result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
            log = result.stdout + result.stderr
            (output / (name + ".log")).write_text(log)
            killed = re.findall(r"^test (\S+) \.\.\. FAILED$", log, re.MULTILINE)
            row = {"mutant": name, "exit": result.returncode, "killing_tests": killed}
            results.append(row)
            print(json.dumps(row), flush=True)
            assert result.returncode == 101 and len(set(killed)) >= 2, row
            path.write_text(originals[path])
    finally:
        for path, source in originals.items():
            path.write_text(source)
        (output / "summary.json").write_text(json.dumps(results, indent=2) + "\n")
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
    (output / "restored-green.log").write_text(result.stdout + result.stderr)
    assert result.returncode == 0
    print(f"PASS {len(results)} mutants, >=2 killing tests each; restored source passes")


if __name__ == "__main__":
    main()
