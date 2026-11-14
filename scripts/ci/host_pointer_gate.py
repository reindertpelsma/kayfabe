#!/usr/bin/env python3
"""Host-pointer gate (gate C's second half, l1_os_shell.md section 9.3): no host CPU address outside
a `*_unsafe.rs` file.

STATUS: LIVE, 2026-11-13. Moved from inline shell in `.github/workflows/ci.yml` (shell is glue,
logic is Python) and widened after security review 1 (F-01 / S1-02): the old gate matched pointer
TYPE names only (`*mut `, `*const `, `NonNull`, `transmute`), so safe code that wrote
`ops.as_mut_ptr() as u64` into an RM control passed it. The gate now also refuses:

- a pointer turned into a wide integer: `.as_ptr()` / `.as_mut_ptr()` followed by
  `as u64|i64|u128|i128` (whitespace and line breaks allowed between them);
- the raw-borrow and address-of forms that make a pointer without naming its type:
  `&raw const`, `&raw mut`, `addr_of!`, `addr_of_mut!`;
- the provenance-exposing API: `expose_provenance`, `with_exposed_provenance`.

Scope limits, deliberate and stated (not an allowlist):

1. `as usize` is NOT matched: three identity-key sites in `kf-util` (and their frozen
   `kayfabe-*` copies) cast `&'static str` pointers to `usize` as atomic keys. They hold no
   dereferenceable address that crosses an ABI, and the crate is outside this change's scope.
   Listed for the coordinator in the PR that added this file.
2. Per FILE, not per visibility (as before).
3. Line comments are stripped (`//` to end of line) before matching, as before, so documents can
   name the forbidden forms. `PhantomData<...>` is erased (a ZST cannot carry an address).

    python3 scripts/ci/host_pointer_gate.py [ROOT]
Exit 0: clean. Exit 1: offenders printed as `path:line: text`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

TYPE_NAMES = re.compile(r"\*mut |\*const |NonNull|transmute")
# A pointer cast to a wide integer, possibly across a line break.
PTR_TO_INT = re.compile(r"\.as_(?:mut_)?ptr\(\)\s*as\s+(?:u64|i64|u128|i128)\b")
ADDRESS_OF = re.compile(
    r"&raw\s+(?:const|mut)\b|\baddr_of(?:_mut)?!|\bexpose_provenance\b|\bwith_exposed_provenance\b"
)
PATTERNS = (TYPE_NAMES, PTR_TO_INT, ADDRESS_OF)

EXCLUDED_DIRS = ("archive", "third_party", "target", "mutants.out")
EXCLUDED_PATHS = ("crates/kayfabe-doorbell",)


def strip(text: str) -> str:
    """Drop line comments and `PhantomData<...>` while keeping line numbers."""
    out = []
    for line in text.split("\n"):
        line = re.sub(r"//.*", "", line)
        line = re.sub(r"PhantomData<[^>]*>", "", line)
        out.append(line)
    return "\n".join(out)


def offenders_in(text: str) -> list[tuple[int, str]]:
    body = strip(text)
    lines = body.split("\n")
    hits: dict[int, str] = {}
    for pat in PATTERNS:
        for m in pat.finditer(body):
            n = body.count("\n", 0, m.start()) + 1
            hits.setdefault(n, lines[n - 1].strip())
    return sorted(hits.items())


def candidate_files(root: Path):
    for f in sorted(root.rglob("*.rs")):
        rel = f.relative_to(root)
        parts = rel.parts
        if f.name.endswith("_unsafe.rs"):
            continue
        if any(p in EXCLUDED_DIRS for p in parts):
            continue
        if any(rel.as_posix().startswith(p + "/") for p in EXCLUDED_PATHS):
            continue
        yield f, rel


def scan(root: Path) -> list[str]:
    found = []
    for f, rel in candidate_files(root):
        try:
            text = f.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            found.append(f"{rel}:0: unreadable: {e}")
            continue
        for n, line in offenders_in(text):
            found.append(f"{rel}:{n}: {line}")
    return found


def main(argv: list[str]) -> int:
    root = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parents[2]
    found = scan(root)
    if not found:
        print("host-pointer gate: clean")
        return 0
    print("\n".join(found))
    print(
        "\n★ HOST-POINTER BREACH — gate C's second half (l1_os_shell.md §9.3).\n"
        "A host CPU address (a pointer type, a pointer cast to a wide integer, or an\n"
        "address-of form) appears in a file that is not named *_unsafe.rs. Two fixes, no third:\n"
        "  - real pointer code -> move it into a *_unsafe.rs module behind a bounded type, or\n"
        "    hand the buffer to kf_linux_raw::Indirect (new/nested/describing), which mints,\n"
        "    bounds and scrubs the address inside the perimeter;\n"
        "  - a comment -> whole-line and trailing // comments are stripped; move prose off code.\n"
        "Never widen the exclusions and never add an allowlist."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
