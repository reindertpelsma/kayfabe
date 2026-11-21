#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf-chan BF-1 mutants (boundary fuzzing 2026-11-18): the span cap of `tmode::resolve_spans` and
the two refusals it keeps apart (`TooManySpans` is refused; `VirtualUnresolved` is waited on).

Each mutant names its file under crates/kf-chan/src, an anchor that must occur exactly once, its
replacement, and the two (or more) tests that must fail. The script refuses a red baseline, a
missing or repeated anchor, a mutant that does not compile, and a named killer that survives.
The mutation is applied in place and restored in `finally` (bytes checked after the restore).

Run: `python3 -B crates/kf-chan/mutants/bf1_mutants.py [name-substring]`.
"""

import os
from pathlib import Path
import re
import subprocess
import sys

CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]

BOUNDED = "a_refused_multi_line_launch_resolves_a_bounded_number_of_spans"
EXACT = "the_span_cap_sits_exactly_above_what_a_bind_could_accept"
WAITED = "a_large_bind_that_needs_a_walk_is_still_waited_not_refused"

MUTANTS = [
    ("cap-dropped", "tmode.rs", "_ if full => return Err(Unresolved::TooManySpans { at }),",
     "_ if false => return Err(Unresolved::TooManySpans { at }),", {BOUNDED, EXACT, WAITED}),
    ("cap-off-by-one", "tmode.rs", "let full = out.len() >= MAX_SPANS;",
     "let full = out.len() > MAX_SPANS;", {BOUNDED, EXACT}),
    ("cap-one-low", "tmode.rs", "pub const MAX_SPANS: usize = MAX_PIECES + 1;",
     "pub const MAX_SPANS: usize = MAX_PIECES;", {EXACT, "refusals_by_name"}),
    ("too-many-is-waited", "tmode.rs",
     "Unresolved::TooManySpans { at } => Refusal::TooManySpans { va, at },",
     "Unresolved::TooManySpans { at } => Refusal::VirtualUnresolved { va, at },", {BOUNDED, EXACT, WAITED}),
    ("uncovered-is-refused", "tmode.rs",
     "Unresolved::Uncovered(at) => Refusal::VirtualUnresolved { va, at },",
     "Unresolved::Uncovered(at) => Refusal::TooManySpans { va, at },", {WAITED, "an_unresolved_operand_is_handed_back_unbound"}),
]

FAILED = re.compile(r"^test (\S+) \.\.\. FAILED$", re.M)
PANIC_FAILED = re.compile(r"^---- (\S+) stdout ----$", re.M)


def run_tests():
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))
    p = subprocess.run(
        ["cargo", "test", "-q", "-p", "kf-chan", "--test", "boundary_bind_cost", "--test", "tmode",
         "--no-fail-fast"],
        cwd=ROOT, env=env, capture_output=True, text=True,
    )
    out = p.stdout + p.stderr
    compiled = "error[E" not in out and "could not compile" not in out
    return p.returncode, compiled, set(FAILED.findall(out)) | set(PANIC_FAILED.findall(out)), out


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    rc, compiled, failed, out = run_tests()
    if rc != 0 or not compiled or failed:
        print(out[-4000:])
        print("BASELINE IS RED: refuse to judge mutants")
        return 2
    bad = 0
    for name, rel, anchor, repl, killers in MUTANTS:
        if only and only not in name:
            continue
        path = CRATE / "src" / rel
        orig = path.read_bytes()
        text = orig.decode()
        n = text.count(anchor)
        if n != 1:
            print(f"ANCHOR {name}: found {n} times in {rel}")
            bad += 1
            continue
        try:
            path.write_text(text.replace(anchor, repl))
            rc, compiled, failed, out = run_tests()
        finally:
            path.write_bytes(orig)
        assert path.read_bytes() == orig, f"{rel} was not restored"
        if not compiled:
            print(f"NOCOMPILE {name}: the mutant does not build (proves nothing)")
            print(out[-2000:])
            bad += 1
        elif len(failed) < 2 or not killers <= failed:
            print(f"SURVIVED {name}: failed={sorted(failed)} named={sorted(killers)}")
            bad += 1
        else:
            print(f"KILLED {name}: {len(failed)} killers: {', '.join(sorted(failed))}")
    print("mutants: " + ("FAIL" if bad else "OK"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
