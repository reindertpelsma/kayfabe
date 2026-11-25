#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the bounded VA-apply phase profiling (STATUS: LIVE, 2026-11-16).

`docs/design/V3_VA_PROFILE.md`: observation-only counters, a log2 histogram, a fixed slow-record
ring with a print budget, the `vat[...]` fields, the split enqueue/delivery stamps and the slot-pool
geometry at a ceiling (`crates/kf-mem/src/vaprof.rs`, `vasmgr.rs`, `batch.rs`,
`crates/kf-host/src/phase.rs`, `crates/kf-qemu/src/mem.rs`, `crates/kf-cuda/src/capacity.rs`).
Each mutant below is a one-line change; each must be killed by at least two independently named
failing tests. Every file touched is overwritten in place and ALWAYS restored (also on an
interrupt); needs a toolchain and the workspace's dependencies.
Run: python3 scripts/ci/va_profile_mutants.py [mutant-name ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
VAPROF = "crates/kf-mem/src/vaprof.rs"
VASMGR = "crates/kf-mem/src/vasmgr.rs"
BATCH = "crates/kf-mem/src/batch.rs"
QMEM = "crates/kf-qemu/src/mem.rs"
CAPACITY = "crates/kf-cuda/src/capacity.rs"
FILES = {}
for rel in (VAPROF, VASMGR, BATCH, QMEM, CAPACITY):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()

MEM = ["-p", "kf-mem", "--lib", "--", "profiling_", "vaprof::"]
QEMU = ["-p", "kf-qemu", "--lib", "--", "profiling_"]
CUDA = ["-p", "kf-cuda", "--lib", "--", "ceiling_geometry"]


def changed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old)
    return rel, FILES[rel].replace(old, new)


CASES = [
    # The inbox wait is taken at the manager's acceptance instead of the worker's enqueue.
    ("q-inbox-from-acceptance", MEM, changed(VASMGR, "at.saturating_duration_since(q)", "at.saturating_duration_since(at.max(q))")),
    # The request latency leaves the inbox wait out.
    ("req-latency-ignores-inbox", MEM, changed(VASMGR, "let req_ns = mgr_ns.saturating_add(p.q_inbox_ns.unwrap_or(0));", "let req_ns = mgr_ns;")),
    # An apply is not counted.
    ("apply-not-counted", MEM, changed(VASMGR, "        pc.aps.add(apply_ns);\n", "")),
    # An apply's phases are not merged into the counters.
    ("phases-not-merged", MEM, changed(VASMGR, "        pc.phases.merge(phases);\n", "")),
    # A window's collect is not counted.
    ("collect-not-counted", MEM, changed(VASMGR, "        self.stats.timing.prof.collect.add(collect_ns);\n", "")),
    # Hidden capacity re-walks are not seen.
    ("cap-retries-not-seen", MEM, changed(VASMGR, "            .capacity_retries()\n            .saturating_sub(batch.retries0);", "            .capacity_retries()\n            .saturating_sub(u64::MAX);")),
    # A ticket does not count its windows.
    ("ticket-windows-not-counted", MEM, changed(VASMGR, "                prof.windows = prof.windows.saturating_add(1);\n", "")),
    # The ring grows past its size (never overwrites).
    ("ring-overflow", MEM, changed(VAPROF, "        if self.len == SLOW_RING {\n            self.overwritten", "        if self.len == SLOW_RING + 1 {\n            self.overwritten")),
    # The ring forgets to count what it overwrote.
    ("ring-overwrite-not-counted", MEM, changed(VAPROF, "            self.overwritten = self.overwritten.saturating_add(1);\n", "")),
    # Histogram buckets off by one.
    ("hist-bucket-off-by-one", MEM, changed(VAPROF, "        63usize\n            .saturating_sub(us.leading_zeros() as usize)", "        64usize\n            .saturating_sub(us.leading_zeros() as usize)")),
    # Percentiles report the bucket's lower bound.
    ("percentile-lower-bound", MEM, changed(VAPROF, "            if cum >= rank {\n                return bucket_upper_us(i);", "            if cum >= rank {\n                return bucket_upper_us(i) / 2;")),
    # The dump budget has no per-boot cap.
    ("dump-no-cap", MEM, changed(VAPROF, "        if self.printed < self.cap && self.tokens > 0 {", "        if self.tokens > 0 {")),
    # The dump budget never refills (a relaunch late in the boot prints nothing).
    ("dump-no-refill", MEM, changed(VAPROF, "        let add = elapsed.checked_div(self.refill_ns).unwrap_or(0);", "        let add = elapsed.checked_div(u64::MAX).unwrap_or(0);")),
    # An old vat field is renamed.
    ("old-field-renamed", MEM, changed(VAPROF, "split_max_ms={} slow_applies={} slow_splits={}\",\n        tm.invals,", "split_max_ms={} slow_applies={} slow_split={}\",\n        tm.invals,")),
    # An old vat field changes meaning (walk_avg_us averages the applies).
    ("old-field-meaning", MEM, changed(VAPROF, "        tm.walks,\n        avg(tm.walk_ns, tm.walks),\n        tm.gpu_us", "        tm.walks,\n        avg(tm.apply_ns, tm.walks),\n        tm.gpu_us")),
    # The ledger's retirement is not recorded.
    ("ledger-retire-not-recorded", MEM, changed(BATCH, "        kf_host::phase::record(\n            kf_host::phase::Phase::LedgerRetire,", "        let _ = (\n            kf_host::phase::Phase::LedgerRetire,")),
    # The span unmap is not timed.
    ("span-unmap-not-timed", MEM, changed(BATCH, "        let r = match via {\n            Some(h) => kf_host::phase::timed(kf_host::phase::Phase::RmUnmap, || {\n                self.vas.unmap_in(h, s, e.saturating_sub(s), defer)\n            }),\n            None => kf_host::phase::timed(kf_host::phase::Phase::RmUnmap, || {\n                self.vas.unmap_range(s, e.saturating_sub(s), defer)\n            }),\n        };\n        if let Some(line) = slow_unmap_line", "        let r = match via {\n            Some(h) => self.vas.unmap_in(h, s, e.saturating_sub(s), defer),\n            None => self.vas.unmap_range(s, e.saturating_sub(s), defer),\n        };\n        if let Some(line) = slow_unmap_line")),
    # The split's enqueue time is stamped at the VA thread's take.
    ("enqueue-stamped-at-take", QEMU, changed(QMEM, "        taken.into_iter().map(|(t, _, p, q)| (t, p, q)).collect()", "        taken\n            .into_iter()\n            .map(|(t, _, p, _q)| (t, p, std::time::Instant::now()))\n            .collect()")),
    # The worker's take is not counted (no resume / e2e).
    ("worker-take-not-counted", QEMU, changed(QMEM, "            self.delivery.e2e.record(ns_between(st.enqueued, now));\n", "")),
    # The geometry looks for the tail at the slot's own offset.
    ("geometry-tail-misplaced", CUDA, changed(CAPACITY, "                    .find(|r| r.off == end)", "                    .find(|r| r.off == o.off)")),
    # In-place growth is judged against the old size only.
    ("geometry-fit-ignores-tail", CUDA, changed(CAPACITY, "if old.is_some_and(|o| u64::from(o.cap) + u64::from(tail_free) >= required) {", "if old.is_some_and(|o| u64::from(o.cap) >= required + 0 * u64::from(tail_free)) {")),
]


def run(overrides, tests):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    env = dict(os.environ, KAYFABE_NO_KVM="1")
    p = subprocess.run(["cargo", "test"] + tests, cwd=ROOT, capture_output=True, text=True, env=env)
    out = p.stdout + p.stderr
    if "could not compile" in out:
        raise SystemExit("the mutant does not compile:\n" + out[-1500:])
    return sorted(set(re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out))), p.returncode


def main():
    try:
        for tests in (MEM, QEMU, CUDA):
            base, rc = run({}, tests)
            assert not base and rc == 0, "baseline is not green: %s %s rc=%s" % (tests[:2], base, rc)
        print("baseline: green")
        weak = []
        wanted = sys.argv[1:]
        for name, tests, (rel, text) in CASES:
            if wanted and name not in wanted:
                continue
            killers, _rc = run({rel: text}, tests)
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
