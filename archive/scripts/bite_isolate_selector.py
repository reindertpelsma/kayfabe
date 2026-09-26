#!/usr/bin/env python3
"""The isolate-SELECTOR bite harness -- remove each guard, WATCH its test go red, restore.

`docs/design/execution_plane_increments.md` E0. The selector is the one place in the tree
where a wrong answer puts a REAL host process behind a guest that did not ask for one, or
-- worse for the evidence -- makes an evidence run and its own negative control
indistinguishable. Every bite below is a real defect shape somebody would plausibly write.

★ WHY THE DEFAULT AND THE REFUSAL ARE BOTH BITTEN. They fail in opposite directions and
only one of them is obvious. A default that moved to `Real` is a security fact (every
build spawns). A near-miss value that quietly becomes `Stillborn` is an EVIDENCE fact: the
2026-08-01 E0 boot pair differs in nothing but `KAYFABE_ISOLATES`, so a typo that silently
selected the refusing plane would have produced a control that looked like a control and
was actually the same run twice.

★ THE FAILURE MODES THIS REPORTS RATHER THAN HIDES -- each looks like success from a
distance:
  - PATTERN NOT UNIQUE -- the guard moved or `cargo fmt` reflowed it, so the bite was never
    applied and the green says nothing. NOT a pass.
  - DID NOT COMPILE -- the removal was rejected by the compiler, not by the test.
    Inconclusive: the test never ran.
  - NON-BITER -- the test passed WITHOUT the guard. That is the finding this exists for.

★★ Files are rewritten and their mtime bumped on both the plant and the restore
(memory: `bite_harness_must_touch_after_restore` -- `cp -a` preserves mtimes and cargo
then serves a stale rlib, manufacturing false non-biters).

⊘ ONE GUARD IS DELIBERATELY NOT BITTEN, and it is a real coverage gap rather than an
oversight: nothing in the pure tests asserts that `IsolatePlane::Real` maps to
`RmMode::Real` rather than `RmMode::Loopback`. `with_the_feature_both_host_planes_build_a_factory`
only asserts `is_ok()`, because the mapping is not observable from a `Box<dyn IsolateFactory>`.
The only instrument that sees it is the live boot, whose witness ASSERTS the spawned child's
`--rm <plane>` argument (`scripts/bench/e0_isolate_witness.sh`). Say so rather than planting
a bite that cannot fire.

⊘ WHAT THESE BITES CANNOT REACH. Every test they drive is over the PURE half
(`isolate_plane_from` / `isolate_factory`). Nothing here reads `KAYFABE_ISOLATES` and
nothing spawns. The env-read arm and the spawn are covered only by the live-boot pair
recorded in `execution_plane_increments.md` §5, and a mutation to `selected_isolate_plane`
itself would be a NON-BITER here by construction -- which is why it is not bitten: a bite
whose non-firing is guaranteed teaches nothing and costs a reader a real investigation.

Usage:  python3 scripts/bite_isolate_selector.py     (from anywhere; paths are repo-relative)
Exit:   0 if every bite fired, 1 otherwise.
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENV = dict(os.environ)

SHIM = "crates/kayfabe-qemu-raw/src/shim.rs"

# (name, file, old, new, test-filter)
BITES = [
    ("BS1 ★★★ the DEFAULT moves to Real -- every build spawns a host process, uninvited",
     SHIM,
     "        None => Ok(IsolatePlane::Stillborn),\n        Some(v) => IsolatePlane::parse(v).ok_or((",
     "        None => Ok(IsolatePlane::Real),\n        Some(v) => IsolatePlane::parse(v).ok_or((",
     "an_unset_selector_is_the_stillborn_plane_master_shipped"),

    ("BS2 ★★★ an unrecognised value quietly becomes Stillborn -- a control that is not one",
     SHIM,
     "        Some(v) => IsolatePlane::parse(v).ok_or((",
     "        Some(v) => Ok(IsolatePlane::parse(v).unwrap_or(IsolatePlane::Stillborn)),\n"
     "        #[allow(unreachable_patterns)]\n"
     "        Some(v) => IsolatePlane::parse(v).ok_or((",
     "a_value_that_is_not_a_plane_name_refuses_rather_than_defaulting"),

    ("BS3 ★★ the parse goes case-insensitive -- `Real` is accepted, and RmMode::parse is not",
     SHIM,
     '    pub fn parse(s: &str) -> Option<IsolatePlane> {\n        match s {',
     '    pub fn parse(s: &str) -> Option<IsolatePlane> {\n        let s: &str = &s.to_lowercase();\n        match s {',
     "a_value_that_is_not_a_plane_name_refuses_rather_than_defaulting"),

    ("BS4 ★★★ a host plane in a build that cannot link one DEGRADES instead of refusing",
     SHIM,
     '        #[cfg(not(feature = "host-isolates"))]\n'
     "        IsolatePlane::Loopback | IsolatePlane::Real => Err((\n"
     "            Status::Unsupported,",
     '        #[cfg(not(feature = "host-isolates"))]\n'
     "        IsolatePlane::Loopback | IsolatePlane::Real => Ok(Box::new(\n"
     "            kayfabe_isolate::StillbornIsolates::new(STILLBORN_WHY),\n"
     "        )),\n"
     '        #[cfg(all(not(feature = "host-isolates"), any()))]\n'
     "        IsolatePlane::Loopback | IsolatePlane::Real => Err((\n"
     "            Status::Unsupported,",
     "without_the_feature_a_host_plane_is_a_named_refusal_not_a_silent_stillborn"),

    ("BS5 ★★ the refusal stops naming the planes -- an operator cannot recover from it",
     SHIM,
     "            \"KAYFABE_ISOLATES does not name an isolate plane: the only values are \\\n"
     "             `stillborn` (the default), `loopback` and `real`. It is not defaulted, \\\n",
     "            \"KAYFABE_ISOLATES is invalid. It is not defaulted, \\\n",
     "a_value_that_is_not_a_plane_name_refuses_rather_than_defaulting"),

]

# The tree's own green, re-run after every restore.
SANITY = "an_unset_selector_is_the_stillborn_plane_master_shipped"


def run(filt):
    p = subprocess.run(
        ["cargo", "test", "-p", "kayfabe-qemu-raw", "--test", "shim_logic",
         "--no-fail-fast", "--", "--exact", filt],
        cwd=ROOT, env=ENV, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def main():
    results = []
    for name, rel, old, new, filt in BITES:
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        if src.count(old) != 1:
            results.append((name, "★ PATTERN NOT UNIQUE (%d hits) -- bite not applied" % src.count(old)))
            continue
        backup = src
        open(path, "w").write(src.replace(old, new))
        os.utime(path, None)
        time.sleep(0.05)
        rc, out = run(filt)
        open(path, "w").write(backup)
        os.utime(path, None)
        time.sleep(0.05)
        if "error[E" in out or "error: could not compile" in out:
            results.append((name, "★ DID NOT COMPILE -- inconclusive"))
        elif "0 passed; 0 failed" in out:
            results.append((name, "★ TEST DID NOT RUN -- filter matched nothing"))
        elif rc != 0:
            results.append((name, "BITES (test went RED)"))
        else:
            results.append((name, "★★★ NON-BITER -- the test passed WITHOUT the guard"))
    rc, _ = run(SANITY)
    print("\n=== ISOLATE-SELECTOR BITE LEDGER ===")
    for n, r in results:
        print(f"  {r:<50} {n}")
    print(f"\nrestored tree sanity check ({SANITY}): {'GREEN' if rc == 0 else 'RED -- RESTORE FAILED'}")
    bad = [n for n, r in results if not r.startswith("BITES")]
    print(f"\n{len(results) - len(bad)}/{len(results)} bites fired")
    sys.exit(1 if bad or rc != 0 else 0)


main()
