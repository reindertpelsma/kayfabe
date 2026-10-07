#!/usr/bin/env python3
"""The reachability bite harness -- remove each fix, WATCH its test go red, restore.

Reachability-on-transition (`docs/design/reachability_on_transition.md`, and
`resume_from_fault.md` §7 step 4) closes five of §6's seven holes. A closure with a test
that passes both with and without its fix is decoration, so this file un-does each fix in
the tree, compiles, runs the ONE test that names it, and reports what happened.

WHY IT IS COMMITTED RATHER THAN RUN ONCE. A bite ledger in a commit message is a claim
about a tree that has since moved. This is re-runnable at any revision, which is the
difference between "eleven bites fired once" and "eleven bites fire".

★ THREE FAILURE MODES IT REPORTS RATHER THAN HIDES, because each of them looks like
success from a distance:
  - PATTERN NOT UNIQUE -- the fix moved or was reformatted, so the bite was never
    applied and the test's green says nothing. NOT a pass.
  - DID NOT COMPILE -- the removal was rejected by the compiler rather than by the test.
    Inconclusive: the test never ran.
  - NON-BITER -- the test passed WITHOUT the fix. That is the finding this exists for.

★★ The file is rewritten and its mtime bumped on both the plant and the restore. `cp -a`
and `shutil.move` preserve mtimes, and cargo then serves a stale rlib -- which manufactures
false non-biters, and has bitten this project before (memory: bite_harness_must_touch_after_restore).

Usage:  python3 scripts/bite_reachability.py     (from anywhere; paths are repo-relative)
Exit:   0 if every bite fired, 1 otherwise.
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENV = dict(os.environ)

BITES = [
    # (name, file, old, new, test-target, test-filter)
    ("hole1 reachability gate",
     "crates/kayfabe-mmu/src/reach.rs",
     "                        if !live {\n                            out.unreachable += 1;\n                            continue;\n                        }",
     "                        if !live {\n                            out.unreachable += 1;\n                        }",
     "reachability", "a_leaf_written_valid_before_its_parent_binds_only_when_the_link_is_published"),

    ("hole2 witness gate",
     "crates/kayfabe-mmu/src/reach.rs",
     "                            if live {\n                                out.unwitnessed += 1;\n                            }\n                            continue;",
     "                            if live {\n                                out.unwitnessed += 1;\n                            }",
     "reachability", "residue_can_make_a_page_reachable_but_never_binds_a_leaf_out_of_it"),

    ("hole3 retirement (no retirement at all)",
     "crates/kayfabe-mmu/src/reach.rs",
     ".filter(|(phys, p)| p.ever_reachable && !reachable.contains(phys))",
     ".filter(|(phys, p)| p.ever_reachable && !reachable.contains(phys) && false)",
     "reachability", "a_pde_clear_retires_the_whole_subtree_and_an_orphan_is_not_retired_with_it"),

    ("hole3 retirement (pass drops the level)",
     "crates/kayfabe-fwd/src/ptdecode.rs",
     "        for phys in &s.retired {\n            vas.pt_meta.remove(phys);\n        }",
     "        for phys in &s.retired {\n            let _ = phys;\n        }",
     "reachability", "the_pass_drops_the_level_of_a_retired_page_so_its_next_write_is_deferred"),

    ("hole3 orphan is NOT retired (ever_reachable dropped)",
     "crates/kayfabe-mmu/src/reach.rs",
     ".filter(|(phys, p)| p.ever_reachable && !reachable.contains(phys))",
     ".filter(|(phys, _p)| !reachable.contains(phys))",
     "reachability", "a_pde_clear_retires_the_whole_subtree_and_an_orphan_is_not_retired_with_it"),

    ("hole4 protection-only change",
     "crates/kayfabe-mmu/src/reach.rs",
     "                Some(&have) if have.same_mapping(want) => {",
     "                Some(&have) if have.same_mapping(want) && false => {",
     "reachability", "a_protection_only_change_is_reported_and_never_silently_unchanged"),

    ("hole5 root audit",
     "crates/kayfabe-mmu/src/reach.rs",
     "        if self.root == pdb.0 & !0xfff {",
     "        if true {",
     "reachability", "a_shadow_whose_root_is_not_the_vas_s_pdb_is_a_loud_refusal"),

    ("hole5 root audit runs in the pass",
     "crates/kayfabe-fwd/src/ptdecode.rs",
     "        if let Err(e) = vas.reach.audit_root(r.task.pdb) {\n            out.reach_faults.push(e);\n            continue;\n        }",
     "        if let Err(e) = vas.reach.audit_root(r.task.pdb) {\n            let _ = e;\n        }",
     "reachability", "the_pass_refuses_a_shadow_whose_root_is_not_the_address_spaces"),

    ("hole6 sparse folded into invalid",
     "crates/kayfabe-mmu/src/walker.rs",
     "            PteDecode::Sparse => out.sparse.push(va),",
     "            PteDecode::Sparse => out.invalid += 1,",
     "reachability", "sparse_is_a_third_state_and_the_three_transitions_differ"),

    ("hole7 the dual slot's second edge",
     "crates/kayfabe-mmu/src/walker.rs",
     "                for e in [Some(edge), also].into_iter().flatten() {",
     "                for e in [Some(edge), also.filter(|_| false)].into_iter().flatten() {",
     "reachability", "a_dual_directory_slot_names_two_sub_tables_and_both_are_followed"),

    ("unbind of a host-published range refused",
     "crates/kayfabe-mmu/src/reach.rs",
     "            Some((_, _, b)) if b.host.is_some() => {",
     "            Some((_, _, b)) if b.host.is_some() && false => {",
     "reachability", "an_unbind_of_a_host_published_range_is_refused_not_performed"),
]


def run(target, filt):
    p = subprocess.run(
        ["cargo", "test", "-p", "kayfabe-tests", "--test", target, "--", "--exact", filt],
        cwd=ROOT, env=ENV, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def main():
    results = []
    for name, rel, old, new, target, filt in BITES:
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        if src.count(old) != 1:
            results.append((name, "★ PATTERN NOT UNIQUE (%d hits) -- bite not applied" % src.count(old)))
            continue
        backup = src
        open(path, "w").write(src.replace(old, new))
        os.utime(path, None)
        time.sleep(0.05)
        rc, out = run(target, filt)
        open(path, "w").write(backup)
        os.utime(path, None)
        time.sleep(0.05)
        if "error[E" in out or "error: could not compile" in out:
            results.append((name, "★ DID NOT COMPILE -- inconclusive"))
        elif rc != 0:
            results.append((name, "BITES (test went RED)"))
        else:
            results.append((name, "★★★ NON-BITER -- the test passed WITHOUT the fix"))
    # the tree must be green again afterwards
    rc, out = run("reachability", "a_leaf_written_valid_before_its_parent_binds_only_when_the_link_is_published")
    print("\n=== BITE LEDGER ===")
    for n, r in results:
        print(f"  {r:<50} {n}")
    print(f"\nrestored tree sanity check: {'GREEN' if rc == 0 else 'RED -- RESTORE FAILED'}")
    bad = [n for n, r in results if not r.startswith("BITES")]
    print(f"\n{len(results) - len(bad)}/{len(results)} bites fired")
    sys.exit(1 if bad else 0)


main()
