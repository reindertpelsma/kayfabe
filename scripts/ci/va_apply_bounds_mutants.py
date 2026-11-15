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
MODEL = "crates/kf-cuda/src/diffmodel.rs"
VASMGR = "crates/kf-mem/src/vasmgr.rs"
FILES = {}
for rel in (BATCH, MODEL, VASMGR):
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
    # --- 2026-11-15 (M4): the report-capacity window (`diffmodel::cut_report`, the spec of kf_diff_emit's
# cut, and the host's treatment of a CUT entry as partial). The kernel's own mutants are in
# scripts/ci/walk_kernel_mutants.py (graded on the real source by kf_cpu_walk).
    ("window-ignores-straddlers", changed(MODEL, "        let straddled = s.iter().any(|r| r.va < v && v < ends(r));", "        let straddled = false;")),
    ("window-allows-one-more", changed(MODEL, "if !straddled && below <= allowed && below >= best.0 {", "if !straddled && below <= allowed + 1 && below >= best.0 {")),
    ("window-is-a-staged-prefix", changed(MODEL, "out.entries[t] = s.iter().filter(|r| r.va < best.1).copied().collect();", "out.entries[t] = s[..allowed.min(s.len())].to_vec();")),
    ("clump-never-refused", changed(MODEL, "    if best.0 == 0 && fbase == 0 {", "    if false {")),
    ("later-entries-not-withheld", changed(MODEL, "        } else if t > f {\n            out.entries[t] = Vec::new();\n        }", "        }")),
    ("cut-flag-never-set", changed(MODEL, "        out.cut[t] = t >= f && out.entries[t].len() < st.len();", "        out.cut[t] = false;")),
    ("host-ignores-the-cut-flag", changed(VASMGR, "                partial: p.vas_flags & (kf_cuda::abi::KFWR_V_PARTIAL | kf_cuda::abi::KFWR_V_CUT)\n                    != 0,", "                partial: p.vas_flags & kf_cuda::abi::KFWR_V_PARTIAL != 0,")),
    ("host-reads-only-the-cut-flag", changed(VASMGR, "                partial: p.vas_flags & (kf_cuda::abi::KFWR_V_PARTIAL | kf_cuda::abi::KFWR_V_CUT)\n                    != 0,", "                partial: p.vas_flags & kf_cuda::abi::KFWR_V_CUT != 0,")),
    ("cut-window-does-not-continue", changed(VASMGR, "                if if withheld { any } else { own } {", "                if false {")),
    ("cut-window-retries-without-progress", changed(VASMGR, "                if if withheld { any } else { own } {", "                if true {")),
    ("no-progress-stays-absence", changed(VASMGR, "                    absent_only.remove(&key);\n                    self.stats.refuse(format!(\n                        \"{key:?}: the walk report was cut", "                    self.stats.refuse(format!(\n                        \"{key:?}: the walk report was cut")),
    # --- review of PR #424: F1 (progress per space), F2 (a budget of windows per request)
    ("progress-judged-per-report", changed(VASMGR, "                if if withheld { any } else { own } {", "                if any {")),
    ("withheld-space-judged-by-its-own-runs", changed(VASMGR, "                if if withheld { any } else { own } {", "                if own {")),
    ("window-budget-not-enforced", changed(VASMGR, "            if *n > self.cut_window_budget {", "            if false {")),
    ("window-budget-off-by-one", changed(VASMGR, "            if *n > self.cut_window_budget {", "            if *n >= self.cut_window_budget {")),
    ("window-count-kept-after-the-request-ends", changed(VASMGR, "            if !again {\n                self.cut_windows.remove(&id);\n            }", "            let _ = again;")),
    # The slow-call line loses its cap.
    ("slow-line-no-cap", changed(BATCH, "(n < SLOW_UNMAP_LOG_CAP).then(|| {", "(n < u32::MAX).then(|| {")),
]

TESTS = ["-p", "kf-mem", "-p", "kf-cuda", "--lib", "--", "chunk", "a_zero_cap", "a_clean_span", "slow_unmap", "a_span_one_entry", "host_call_amplification", "cut_report", "a_cut_report", "draining_a_diff", "windowed_apply", "a_staged_order_prefix", "a_split_over", "the_split_verdict", "an_invalidate_over", "a_window_carries", "a_clump_larger", "a_space_whose", "an_invalidate_whose", "one_persistently", "a_guest_that", "the_budget_is", "a_split_waits_for", "an_invalidate_waits_for", "a_chain_of", "a_cut_entry", "a_withheld", "a_capacity_withheld", "a_refused_map_in_a_window", "a_window_in_which", "an_invalidate_over_a_window", "hostile_arguments"]


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
