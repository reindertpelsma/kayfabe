#!/usr/bin/env python3
"""Mutation checks for scripts/ci/host_pointer_gate.py: every mutant needs two killers in
scripts/ci/test_host_pointer_gate.py. Each mutant is applied to a temporary copy of the gate; the
tests import that copy. Stdlib only; GPU-free.

    python3 -B scripts/ci/host_pointer_gate_mutants.py [substring-of-mutant-name]
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
GATE = HERE / "host_pointer_gate.py"
TESTS = HERE / "test_host_pointer_gate.py"

# (name, old text (exactly once), new text)
MUTANTS = [
    (
        "no-pointer-to-int-pattern",
        "PATTERNS = (TYPE_NAMES, PTR_TO_INT, ADDRESS_OF)",
        "PATTERNS = (TYPE_NAMES, ADDRESS_OF)",
    ),
    (
        "pointer-to-int-same-line-only",
        r'PTR_TO_INT = re.compile(r"\.as_(?:mut_)?ptr\(\)\s*as\s+(?:u64|i64|u128|i128)\b")',
        r'PTR_TO_INT = re.compile(r"\.as_(?:mut_)?ptr\(\)[ \t]*as[ \t]+(?:u64|i64|u128|i128)\b")',
    ),
    (
        "pointer-to-int-u64-only",
        r"(?:u64|i64|u128|i128)\b",
        r"(?:u64)\b",
    ),
    (
        "as-mut-ptr-only",
        r'r"\.as_(?:mut_)?ptr\(\)',
        r'r"\.as_mut_ptr\(\)',
    ),
    (
        "no-address-of-pattern",
        "PATTERNS = (TYPE_NAMES, PTR_TO_INT, ADDRESS_OF)",
        "PATTERNS = (TYPE_NAMES, PTR_TO_INT)",
    ),
    (
        "no-type-name-pattern",
        "PATTERNS = (TYPE_NAMES, PTR_TO_INT, ADDRESS_OF)",
        "PATTERNS = (PTR_TO_INT, ADDRESS_OF)",
    ),
    (
        "comments-not-stripped",
        '        line = re.sub(r"//.*", "", line)\n',
        "",
    ),
    (
        "unsafe-files-scanned",
        '        if f.name.endswith("_unsafe.rs"):\n            continue\n',
        "",
    ),
    (
        "tests-dir-skipped",
        '        if any(p in EXCLUDED_DIRS for p in parts):',
        '        if any(p in EXCLUDED_DIRS + ("tests",) for p in parts):',
    ),
    (
        "main-always-zero",
        "    print(\"\\n\".join(found))\n",
        "    print(\"\\n\".join(found))\n    return 0\n",
    ),
    (
        "line-number-off-by-one",
        '            n = body.count("\\n", 0, m.start()) + 1',
        '            n = body.count("\\n", 0, m.start())',
    ),
]


def killers(work: Path) -> list:
    out = subprocess.run(
        [sys.executable, "-B", str(work / "test_host_pointer_gate.py"), "-v"],
        capture_output=True,
        text=True,
        cwd=work,
        env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
        timeout=300,
    )
    dead = []
    for line in out.stderr.splitlines():
        if line.endswith("... FAIL") or line.endswith("... ERROR"):
            dead.append(line.split(" ")[0])
    if out.returncode != 0 and not dead:
        dead.append("<crash>")
    return dead


def main() -> int:
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    rows, bad = [], 0
    for name, old, new in MUTANTS:
        if only and only not in name:
            continue
        src = GATE.read_text()
        if src.count(old) != 1:
            print(f"{name}: anchor not found exactly once", file=sys.stderr)
            return 2
        with tempfile.TemporaryDirectory(prefix="hpgate-mut-") as d:
            work = Path(d)
            (work / "host_pointer_gate.py").write_text(src.replace(old, new))
            shutil.copy(TESTS, work / "test_host_pointer_gate.py")
            # The real-tree test reads HERE.parents[1]; point it at the repository.
            t = (work / "test_host_pointer_gate.py").read_text()
            t = t.replace("gate.scan(HERE.parents[1])", f"gate.scan(Path({str(HERE.parents[1])!r}))")
            (work / "test_host_pointer_gate.py").write_text(t)
            k = killers(work)
        rows.append({"mutant": name, "killers": k, "count": len(k)})
        print(f"{name}: {len(k)} killers", file=sys.stderr, flush=True)
        bad += len(k) < 2
    print(json.dumps(rows, indent=1))
    if bad:
        print(f"{bad} mutant(s) with fewer than 2 killers", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
