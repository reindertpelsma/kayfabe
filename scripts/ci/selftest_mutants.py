#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""E3c's known positive (V3_SEC_PERIMETER.md §3.3, §3.6): a perimeter fn whose OK row cites a
VACUOUS test must yield MISSED mutants and fail the mutation verdict; the same fn with a test
that pins its behaviour must pass. Needs cargo and cargo-mutants (nightly job only).
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
K = "un" + "safe"
SRC = """\
/// Reads byte `i`, refusing an out-of-range index.
pub fn checked(x: &[u8], i: usize) -> Option<u8> {
    if i >= x.len() {
        return None;
    }
    Some(x[i])
}
#[cfg(test)]
mod tests {
    #[test]
    fn exercise() {
        BODY
    }
}
"""
VACUOUS = "let _ = super::checked(&[1], 0);"
STRONG = "assert_eq!(super::checked(&[1], 1), None);\n        assert_eq!(super::checked(&[7], 0), Some(7));"
TABLE = """\
# fixture

## src/a_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `checked` | pub | safe fn | range | range=t:src/a_unsafe.rs::exercise | OK |
"""


def run(tmp: Path, name: str, body: str) -> tuple[int, str]:
    root = tmp / name
    (root / "src").mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname = "mfx"\nversion = "0.1.0"\nedition = "2024"\n[workspace]\n')
    (root / "src/lib.rs").write_text("mod a_unsafe;\npub use a_unsafe::checked;\n")
    (root / "src/a_unsafe.rs").write_text(SRC.replace("BODY", body))
    (root / "docs/design").mkdir(parents=True)
    (root / "docs/design/PERIMETER_EXPORTS.md").write_text(TABLE)
    out = tmp / f"{name}.out"
    env = {**os.environ, "CARGO_TARGET_DIR": str(tmp / f"{name}.target")}
    subprocess.run(["cargo", "mutants", "--no-shuffle", "-o", str(out)], cwd=root, env=env, capture_output=True)
    r = subprocess.run([sys.executable, str(HERE / "perimeter.py"), "--root", str(root), "--config",
                        str(HERE / "perimeter.toml"), "mutants", "--outcomes",
                        str(out / "mutants.out" / "outcomes.json")], capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="kf-mutants-selftest-") as t:
        tmp = Path(t)
        rc1, out1 = run(tmp, "vacuous", VACUOUS)
        rc2, out2 = run(tmp, "strong", STRONG)
    ok1 = rc1 != 0 and "E3c " in out1 and "MISSED" in out1
    ok2 = rc2 == 0 and "PERIMETER_MUTANTS findings=0" in out2
    print(f"{'ok  ' if ok1 else 'FAIL'} a vacuous test leaves MISSED mutants and fails E3c: rc={rc1}")
    print(f"{'ok  ' if ok2 else 'FAIL'} a test that pins the behaviour passes: rc={rc2}")
    if not (ok1 and ok2):
        print(out1[-2000:])
        print(out2[-2000:])
    print(f"MUTANTS_SELFTEST failed={int(not ok1) + int(not ok2)}")
    return 0 if ok1 and ok2 else 1


if __name__ == "__main__":
    sys.exit(main())
