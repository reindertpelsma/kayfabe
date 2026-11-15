#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the host unmap-call cap (STATUS: LIVE, 2026-11-15).

`crates/kf-mem/src/batch.rs` cuts one owned span into host unmap calls of at most
`UNMAP_CALL_MAX_ENTRIES` ledger entries (`BatchedVas::unmap_owned`). Each mutant below is a one-line
change of that code; each must be killed by at least two independently named failing tests of
`kf-mem` (`sim::adversarial::*`, `batch::tests::*`). Every file touched is overwritten in place and
ALWAYS restored (also on an interrupt); needs a toolchain and the workspace's dependencies.
Run: python3 scripts/ci/va_apply_bounds_mutants.py [mutant-name ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
BATCH = "crates/kf-mem/src/batch.rs"
FILES = {}
for rel in (BATCH,):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()


def changed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old)
    return rel, FILES[rel].replace(old, new)


CASES = [
    # The cap is ignored: one call per span again.
    ("cap-ignored", changed(BATCH, "if *oe == cs && *via == m.via && *n < cap =>", "if *oe == cs && *via == m.via =>")),
    # Off by one: a call covers cap + 1 entries.
    ("cap-off-by-one", changed(BATCH, "if *oe == cs && *via == m.via && *n < cap =>", "if *oe == cs && *via == m.via && *n <= cap =>")),
    # The ledger is not cut when a chunk lands.
    ("ledger-not-cut", changed(BATCH, "        r?;\n        self.cut_own(s, e);", "        r?;")),
    # A chunk that failed is recorded as gone anyway (a half-applied chunk the ledger hides).
    ("failed-chunk-cut-anyway", changed(BATCH, "        r?;\n        self.cut_own(s, e);\n        self.retired(s, e.saturating_sub(s));\n        Ok(())", "        self.cut_own(s, e);\n        self.retired(s, e.saturating_sub(s));\n        r")),
    # The span's tail chunk is never issued.
    ("last-chunk-dropped", changed(BATCH, "        if let Some((lo, hi, via, n)) = open {\n            self.unmap_span((lo, hi, via), n, defer)?;\n        }", "        let _ = open;")),
    # The default cap changes.
    ("default-cap-changed", changed(BATCH, "pub const UNMAP_CALL_MAX_ENTRIES: usize = 4096;", "pub const UNMAP_CALL_MAX_ENTRIES: usize = 4097;")),
    # The slow-call line loses its threshold.
    ("slow-line-no-threshold", changed(BATCH, "    if elapsed < std::time::Duration::from_millis(SLOW_UNMAP_CALL_MS) {\n        return None;\n    }", "    let _ = SLOW_UNMAP_CALL_MS;")),
    # The slow-call line loses its cap.
    ("slow-line-no-cap", changed(BATCH, "(n < SLOW_UNMAP_LOG_CAP).then(|| {", "(n < u32::MAX).then(|| {")),
]

TESTS = ["-p", "kf-mem", "--lib", "--", "chunk", "a_zero_cap", "a_clean_span", "slow_unmap", "a_span_one_entry", "host_call_amplification"]


def run(overrides):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    p = subprocess.run(["cargo", "test"] + TESTS, cwd=ROOT, capture_output=True, text=True)
    out = p.stdout + p.stderr
    if "could not compile" in out:
        raise SystemExit("the mutant does not compile:\n" + out[-800:])
    return sorted(set(re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out))), p.returncode


def main():
    try:
        base, rc = run({})
        assert not base and rc == 0, "baseline is not green: %s rc=%s" % (base, rc)
        print("baseline: green")
        weak = []
        wanted = sys.argv[1:]
        for name, (rel, text) in CASES:
            if wanted and name not in wanted:
                continue
            killers, _rc = run({rel: text})
            with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
                f.write(FILES[rel])
            print("%-28s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(CASES), len(weak)))
        return 1 if weak else 0
    finally:
        for rel, text in FILES.items():
            with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
                f.write(text)


if __name__ == "__main__":
    sys.exit(main())
