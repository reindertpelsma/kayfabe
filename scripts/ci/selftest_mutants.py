#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""E3c's known positives (V3_SEC_PERIMETER.md §3.3, §3.6). Each case runs cargo-mutants over a
one-file fixture and the mutation verdict over its outcomes, and must end the way it says:

- vacuous: an OK row whose test asserts nothing leaves MISSED mutants: fails.
- strong: the same row with a test that pins the behaviour: passes (the control), also under
  --require-caught.
- helper: the row's own body is pinned, but its check lives in a private same-file helper the
  test does not pin: the helper's missed mutants are CHARGED to the row: fails.
- validates: the same, with the helper in a VALIDATES file (bounds.rs): fails.
- zero_caught: an OK row no mutant can touch, under --require-caught: fails (zero caught).
- baseline: a test that fails unmutated, under --require-caught: fails (no Success baseline).

Needs cargo and cargo-mutants (the perimeter-mutants workflow's selftest job).
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = """\
/// Reads byte `i`, refusing an out-of-range index.
pub fn checked(x: &[u8], i: usize) -> Option<u8> {
    if !HELPER(x.len(), i) {
        return None;
    }
    Some(x[i])
}
/// Nothing to mutate.
pub fn noop() {}
fn in_range(len: usize, i: usize) -> bool {
    i < len
}
#[cfg(test)]
mod tests {
    #[test]
    fn exercise() {
        super::noop();
        BODY
    }
}
"""
BOUNDS = "/// In range.\npub fn in_range(len: usize, i: usize) -> bool {\n    i < len\n}\n"
VACUOUS = "let _ = super::checked(&[1], 0);"
STRONG = ("assert_eq!(super::checked(&[1], 1), None);\n        assert_eq!(super::checked(&[7], 0), Some(7));\n"
          "        assert_eq!(super::checked(&[7, 8], 1), Some(8));\n        assert_eq!(super::checked(&[7], 2), None);")
# pins `checked`'s own body (None vs Some, the `!`) but not the helper's `<` (i == len is never asked)
OWN_ONLY = "assert_eq!(super::checked(&[7], 0), Some(7));\n        assert_eq!(super::checked(&[7], 5), None);"
ROW = "| `checked` | pub | safe fn | range | range=t:src/a_unsafe.rs::exercise | OK |"
NOOP = "| `noop` | pub | safe fn | none | none=t:src/a_unsafe.rs::exercise | OK |"
VD = "\n## Validation dependencies\n\n| file | role | why |\n|---|---|---|\n| `src/bounds.rs` | VALIDATES | in_range |\n"


def run(tmp: Path, name: str, body: str, rows: list[str], helper: str = "in_range",
        require: bool = False) -> tuple[int, str]:
    root = tmp / name
    (root / "src").mkdir(parents=True)
    (root / "Cargo.toml").write_text('[package]\nname = "mfx"\nversion = "0.1.0"\nedition = "2024"\n[workspace]\n')
    (root / "src/lib.rs").write_text("mod a_unsafe;\npub mod bounds;\npub use a_unsafe::{checked, noop};\n")
    (root / "src/a_unsafe.rs").write_text(SRC.replace("HELPER", helper).replace("BODY", body))
    (root / "src/bounds.rs").write_text(BOUNDS)
    (root / "docs/design").mkdir(parents=True)
    (root / "docs/design/PERIMETER_EXPORTS.md").write_text(
        "# fixture\n\n## src/a_unsafe.rs\n\n| item | vis | kind | checks | tests | status |\n|---|---|---|---|---|---|\n"
        + "\n".join(rows) + "\n" + VD)
    out = tmp / f"{name}.out"
    env = {**os.environ, "CARGO_TARGET_DIR": str(tmp / f"{name}.target")}
    subprocess.run(["cargo", "mutants", "--no-shuffle", "-o", str(out)], cwd=root, env=env, capture_output=True)
    cmd = [sys.executable, str(HERE / "perimeter.py"), "--root", str(root), "--config", str(HERE / "perimeter.toml"),
           "mutants", "--outcomes", str(out / "mutants.out" / "outcomes.json")]
    if require:
        cmd += ["--require-caught", "--crate", "src"]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


# (name, test body, rows, helper, --require-caught, must pass, marker that must appear)
CASES = [
    ("vacuous", VACUOUS, [ROW], "in_range", False, False, "MISSED"),
    ("strong", STRONG, [ROW], "in_range", True, True, "PERIMETER_MUTANTS findings=0"),
    ("helper", OWN_ONLY, [ROW], "in_range", False, False, "in `in_range`, src/a_unsafe.rs"),
    ("validates", OWN_ONLY, [ROW], "crate::bounds::in_range", False, False, "in `in_range`, src/bounds.rs"),
    ("zero_caught", STRONG, [ROW, NOOP], "in_range", True, False, "`noop` has ZERO caught mutants"),
    ("baseline", "assert_eq!(super::checked(&[1], 0), None);", [ROW], "in_range", True, False,
     "the unmutated baseline is"),
]


def main() -> int:
    failed = []
    with tempfile.TemporaryDirectory(prefix="kf-mutants-selftest-") as t:
        tmp = Path(t)
        for name, body, rows, helper, require, must_pass, marker in CASES:
            rc, out = run(tmp, name, body, rows, helper, require)
            ok = (rc == 0) == must_pass and marker in out
            print(f"{'ok  ' if ok else 'FAIL'} {name}: rc={rc}")
            if not ok:
                failed.append(name)
                print(out[-2500:])
    print(f"MUTANTS_SELFTEST cases={len(CASES)} failed={len(failed)} {failed}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
