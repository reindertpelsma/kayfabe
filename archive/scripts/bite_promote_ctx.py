#!/usr/bin/env python3
"""The `GPU_PROMOTE_CTX` bite harness -- remove each fix, WATCH its test go red, restore.

`docs/design/gpu_promote_ctx.md` §3 names SEVEN defects in the C artifact's promote-ctx
handler, two of them guest-reachable memory-safety bugs, and §9.4 records each one as
subtracted. A subtraction whose test passes with the fix AND without it is decoration.
This file un-does each fix in the tree, compiles, runs the ONE test that names it, and
reports what happened.

WHY IT IS COMMITTED RATHER THAN RUN ONCE. §9.4's "each has a test that was seen to fail
when the fix was poisoned" is a claim about a tree that has since moved (it was written at
17 gate steps; the floor is 19 now). This is re-runnable at any revision, which is the
difference between "the bites fired once" and "the bites fire". Same argument, same shape
and same three reported failure modes as `bite_reachability.py`.

★ THREE FAILURE MODES IT REPORTS RATHER THAN HIDES, because each looks like success from
a distance:
  - PATTERN NOT UNIQUE -- the fix moved or was reformatted, so the bite was never applied
    and the test's green says nothing. NOT a pass.
  - DID NOT COMPILE -- the removal was rejected by the compiler rather than by the test.
    Inconclusive: the test never ran.
  - NON-BITER -- the test passed WITHOUT the fix. That is the finding this exists for.

★★ The file is rewritten and its mtime bumped on both the plant and the restore. `cp -a`
and `shutil.move` preserve mtimes, and cargo then serves a stale rlib -- which manufactures
false non-biters, and has bitten this project before (memory: bite_harness_must_touch_after_restore).

★ Every bite is a REAL DEFECT SHAPE, not a random mutation: D1 plants the C's literal
clamp of 64, D2 plants its 32-bit read reaching the flag bytes, D4 plants its collapse of
the aperture to one bit, D7 plants its write-back over the caller's params, and the
`rpc_bound` bite plants §6.1's trap verbatim. A bite that is not a shape someone could
plausibly write proves less than one that is.

Usage:  python3 scripts/bite_promote_ctx.py     (from anywhere; paths are repo-relative)
Exit:   0 if every bite fired, 1 otherwise.
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENV = dict(os.environ)

VIEW = "crates/kayfabe-abi/src/view.rs"
VERS = "crates/kayfabe-abi/src/versions.rs"
CTRL = "crates/kayfabe-abi/src/generated/ctrl.rs"
PROM = "crates/kayfabe-core/src/promote.rs"
GPU = "crates/kayfabe-core/src/gpu.rs"
RMRPC = "crates/kayfabe-rmrpc/src/lib.rs"
DEV = "crates/kayfabe-rt/src/device.rs"

# (name, file, old, new, test-filter) -- every test lives in `tests/tests/promote_ctx.rs`.
BITES = [
    # ── §3's seven C defects ────────────────────────────────────────────────────────
    ("D1 SECURITY entryCount clamped to the C's literal 64",
     VERS,
     "        if declared as usize > MAX_PROMOTE_ENTRIES {",
     "        if declared as usize > 64 {",
     "the_entry_count_bound_is_16_and_it_is_refused_not_clamped"),

    ("D2 bufferId read wide enough to reach the flag bytes",
     CTRL,
     "            buffer_id: u16_at(bytes, 28)?,",
     "            buffer_id: u16_at(bytes, 30)?,",
     "the_flag_bytes_cannot_reach_buffer_id"),

    ("D3 promote-only entries swallowed silently instead of counted",
     VIEW,
     "                PromoteEntry::PromoteOnly { .. } => c.promote_only += 1,",
     "                PromoteEntry::PromoteOnly { .. } => {}",
     "wire_bytes_reach_the_address_table"),

    ("D4 aperture collapsed to one bit, so 3 is accepted",
     VIEW,
     "        match phys_attr & 0x3 {",
     "        match phys_attr & 0x1 {",
     "the_aperture_is_total_and_three_is_refused_by_name"),

    ("D5 the acting client is not checked against the address space's owner",
     PROM,
     "    if !proc.client_values().contains(&p.client) {",
     "    if !proc.client_values().contains(&p.client) && false {",
     "a_foreign_acting_client_cannot_promote_into_another_procs_address_space"),

    ("D6 the core's own range bound stops refusing",
     PROM,
     "    if p.ranges.len() > MAX_PROMOTED_RANGES {",
     "    if p.ranges.len() > MAX_PROMOTED_RANGES * 8 {",
     "more_ranges_than_the_core_bound_is_refused"),

    ("D7 SECURITY a Case-2 ACK writes back into the caller's params",
     DEV,
     "            if let ControlRoute::AckOnly = ack {\n"
     "                return Ok(ControlRoute::AckOnly);\n"
     "            }",
     "            if let ControlRoute::AckOnly = ack {\n"
     "                if !payload.is_empty() {\n"
     "                    payload[0] ^= 0xff;\n"
     "                }\n"
     "                return Ok(ControlRoute::AckOnly);\n"
     "            }",
     "a_case2_ack_writes_nothing_back"),

    # ── §2's protocol: the three legitimate wire states ─────────────────────────────
    ("★★★ 'NOT SUPPLIED' read as physical zero (§2.3)",
     VIEW,
     "    if e.gpu_virt_addr != 0 && e.size != 0 {",
     "    if e.gpu_virt_addr != 0 {",
     "the_three_states_are_classified_by_content_not_dropped"),

    ("bNonmapped no longer dominates the VALUE",
     VIEW,
     "    if e.b_nonmapped != 0 {",
     "    if e.b_nonmapped != 0 && e.gpu_virt_addr == 0 {",
     "b_nonmapped_is_never_promotable_however_plausible_the_values"),

    ("the legacy shape refused on ALL three fields instead of ANY",
     VERS,
     "        if h.h_virt_memory != 0 || h.virt_address != 0 || h.size != 0 {",
     "        if h.h_virt_memory != 0 && h.virt_address != 0 && h.size != 0 {",
     "the_legacy_shape_is_refused_by_name"),

    ("the guest's declared paramsSize checked loosely instead of exactly",
     RMRPC,
     "        && declared != expected",
     "        && declared > expected",
     "a_declared_size_that_is_not_560_is_refused_with_both_numbers"),

    # ── §6's two traps, and the join's own laws ────────────────────────────────────
    ("★★ §6.1 the promote source files its bindings under rpc_bound",
     PROM,
     "        vas.promote_bound.insert(r.va.0);",
     "        vas.rpc_bound.insert(r.va.0);",
     "promote_bindings_survive_a_subsequent_spine_apply"),

    ("typed resolution: an object of the wrong kind is accepted",
     PROM,
     "    ) {\n        return Err(PromoteFault::NotAContextObject {",
     "    ) && false {\n        return Err(PromoteFault::NotAContextObject {",
     "the_routing_refusals_are_named"),

    ("the owning proc is GUESSED when the PDB is not in the routing map",
     PROM,
     "    let proc = *spine\n        .by_pdb\n        .get(&(gpu, pdb))",
     "    let proc = *spine\n        .by_pdb\n        .iter()\n        .next()\n        .map(|(_, v)| v)",
     "two_procs_identical_vas_land_in_two_tables"),

    ("self-overlap inside one promotion stops being refused",
     PROM,
     "            if overlap {",
     "            if overlap && false {",
     "a_promotion_that_would_half_apply_is_refused_whole"),

    ("a zero-length range stops being malformed",
     PROM,
     "        if r.len == 0 || r.va.0.checked_add(r.len).is_none() {",
     "        if r.va.0.checked_add(r.len).is_none() {",
     "a_promotion_that_would_half_apply_is_refused_whole"),

    ("the identical re-promote stops being the one accepted overlap",
     PROM,
     "            if !identical {",
     "            if !identical || true {",
     "an_identical_repromote_is_idempotent"),

    ("the ownership index is accreted instead of derived",
     GPU,
     "        self.ctx_vas.clear();",
     "        // self.ctx_vas.clear();",
     "a_freed_context_object_leaves_the_ownership_index"),
]

# The tree's own green, re-run after every restore. Deliberately the MEAN test: it is the
# one that drives both lock modes through the L1 shell.
SANITY = "mean_promote_through_the_shell"


def run(filt):
    p = subprocess.run(
        ["cargo", "test", "-p", "kayfabe-tests", "--test", "promote_ctx",
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
            results.append((name, "★★★ NON-BITER -- the test passed WITHOUT the fix"))
    rc, _ = run(SANITY)
    print("\n=== PROMOTE-CTX BITE LEDGER ===")
    for n, r in results:
        print(f"  {r:<50} {n}")
    print(f"\nrestored tree sanity check ({SANITY}): {'GREEN' if rc == 0 else 'RED -- RESTORE FAILED'}")
    bad = [n for n, r in results if not r.startswith("BITES")]
    print(f"\n{len(results) - len(bad)}/{len(results)} bites fired")
    sys.exit(1 if bad or rc != 0 else 0)


main()
