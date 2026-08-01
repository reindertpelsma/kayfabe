#!/usr/bin/env python3
"""The E0b/E1 bite harness -- remove each guard, WATCH its test go red, restore.

`docs/design/execution_plane_increments.md` E0b (the spawn is caused by the GUEST, not by
`Gpu::realize`) and E1 (a refusing isolate is visible BY NAME and BY KIND).

★ WHY THESE TWO INCREMENTS SHARE A HARNESS. They fail into each other. E0b makes "no
isolate exists" a legal state, which is precisely the state E1 has to be able to describe:
before E0b an isolate always existed, so `materialized == 0` could not happen and needed no
name; after E0b it can, and a build that lost E1's census would report that state as the
same silence as a failed spawn. A bite on one that is caught only by the other's test would
be a finding, and this is where it would show.

★ THE FAILURE MODES THIS REPORTS RATHER THAN HIDES -- each looks like success from a
distance:
  - PATTERN NOT UNIQUE -- the guard moved or `cargo fmt` reflowed it, so the bite was never
    applied and the green says nothing. NOT a pass.
  - DID NOT COMPILE -- the removal was rejected by the compiler, not by the test.
    Inconclusive: the test never ran.
  - NON-BITER -- the test passed WITHOUT the guard. That is the finding this exists for.

★★ Files are rewritten and their mtime bumped on both the plant and the restore
(memory: `bite_harness_must_touch_after_restore` -- `cp -a` preserves mtimes and cargo then
serves a stale rlib, manufacturing false non-biters).

⊘ WHAT THESE BITES CANNOT REACH, said rather than implied:

  1. **The ATTRIBUTION.** Every test driven here observes `Gpu::isolate_census()`, which the
     code under test writes. It can say a spawn happened at event N and never that a spawn
     happened BECAUSE OF the guest. Only `scripts/bench/e0_isolate_witness.sh` can, because
     it stamps host `/proc` sightings against `boot_capture.sh`'s own phase lines. A
     mutation that made the device LIE about the census would be caught here; a mutation
     that moved the spawn back to realize-time and also moved the counter would not, and
     the boot is what covers that.
  2. **The C shell's print.** `qemu/hw/misc/nvkvm/nvkvm.c`'s `info_report` is not compiled
     by `cargo`. Deleting it turns nothing red in this harness; the boot's own witness
     greps for the line and says so when it is absent.
  3. **The `SpawnFailed` producer on real hardware.** `HostIsolate::refusal` is bitten
     against its unit test, which builds a stillborn isolate directly. The arm where a real
     `clone` really fails is the bench run `e0bfail1`
     (`sysctl -w user.max_user_namespaces=0`), not this file.

Usage:  python3 scripts/bite_lazy_isolate.py     (from anywhere; paths are repo-relative)
Exit:   0 if every bite fired, 1 otherwise.
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENV = dict(os.environ)

GPU = "crates/kayfabe-core/src/gpu.rs"
ISO = "crates/kayfabe-isolate/src/lib.rs"
HOST = "crates/kayfabe-isolate-host/src/isolate.rs"

# (name, file, old, new, package, test-target, test-filter)
BITES = [
    ("BL1 ★★★ realize spawns EAGERLY again -- the exact E0 defect, restored",
     GPU,
     "        spine.ensure_proc_arena(&mut system, GpuId::ZERO)?;",
     "        spine.ensure_proc_arena(&mut system, GpuId::ZERO)?;\n"
     "        spine.materialize_isolate(&mut system, GpuId::ZERO);",
     "kayfabe-tests", "isolate_spawn_is_guest_caused",
     "realizing_a_device_materializes_no_isolate_at_all"),

    ("BL2 ★★★ the lazy spawn is dropped -- a device that is driven never gets an isolate",
     GPU,
     "                if self.materialize_isolate(system, GpuId::ZERO) {\n"
     "                    self.isolates_materialized = self.isolates_materialized.saturating_add(1);\n"
     "                }",
     "                let _ = &system;",
     "kayfabe-tests", "isolate_spawn_is_guest_caused",
     "the_first_accepted_guest_event_materializes_the_system_isolate"),

    ("BL3 ★★ the spawn moves to the REFUSED arm too -- garbage buys a guest a host process",
     GPU,
     "            Err(e) => {\n"
     "                // Undo: restore the last-good graph and re-derive from it.",
     "            Err(e) => {\n"
     "                self.materialize_isolate(system, GpuId::ZERO);\n"
     "                // Undo: restore the last-good graph and re-derive from it.",
     "kayfabe-tests", "isolate_spawn_is_guest_caused",
     "a_refused_event_materializes_nothing"),

    ("BL4 ★★★ E1: a failed HOST spawn reports NoPlane -- gap 7, reopened by one word",
     HOST,
     "                kind: kayfabe_isolate::RefusalKind::SpawnFailed,",
     "                kind: kayfabe_isolate::RefusalKind::NoPlane,",
     "kayfabe-isolate-host", None,
     "a_failed_host_isolate_is_distinguishable_from_a_deliberately_planeless_one"),

    ("BL5 ★★★ E1: the shipped plane-less build reports SpawnFailed -- an operator debugs a "
     "host that is fine",
     ISO,
     "            kind: RefusalKind::NoPlane,\n            why: self.why,",
     "            kind: RefusalKind::SpawnFailed,\n            why: self.why,",
     "kayfabe-tests", "isolate_spawn_is_guest_caused",
     "the_shipped_default_plane_reports_no_plane_and_never_a_failure"),

    ("BL6 ★★ E1: a LIVE isolate reports a refusal -- every healthy boot prints a failure",
     HOST,
     "        self.spawn_error\n            .as_deref()",
     "        Some(self.spawn_error.as_deref().unwrap_or(\"\"))\n            .as_deref()",
     "kayfabe-isolate-host", None,
     "a_live_isolate_reports_no_refusal_even_after_retire"),

    ("BL7 ★★ E1: the census loses the SpawnFailed precedence -- the actionable sentence is "
     "dropped for the expected one",
     ISO,
     "            Some((RefusalKind::NoPlane, _)) => r.kind == RefusalKind::SpawnFailed,",
     "            Some((RefusalKind::NoPlane, _)) => false,",
     "kayfabe-tests", "isolate_spawn_is_guest_caused",
     "a_spawn_failure_outranks_a_missing_plane_in_either_order"),

    ("BL8 ★★ E1: the census stops counting spawn failures apart from plane-less refusals",
     ISO,
     "            RefusalKind::SpawnFailed => self.spawn_failed = self.spawn_failed.saturating_add(1),",
     "            RefusalKind::SpawnFailed => self.no_plane = self.no_plane.saturating_add(1),",
     "kayfabe-isolate-host", None,
     "a_failed_host_isolate_is_distinguishable_from_a_deliberately_planeless_one"),

    ("BL9 ★★★ E0b: the per-PROCESS isolate is collapsed onto the system one -- #14, restored "
     "at the seam the owner ruled must not be rewritten",
     GPU,
     "                p.isolates.entry(gpu).or_insert_with(|| {\n"
     "                    spawned = true;\n"
     "                    IsolateBox::new(isolates.spawn(IsolateId::new(pid.0, gpu)))\n"
     "                });",
     "                p.isolates.entry(gpu).or_insert_with(|| {\n"
     "                    spawned = true;\n"
     "                    IsolateBox::new(isolates.spawn(IsolateId::new(Gpu::SYSTEM_PROC.0, gpu)))\n"
     "                });",
     "kayfabe-tests", "isolate_spawn_is_guest_caused",
     "two_guest_processes_get_their_own_isolates_and_none_exists_at_realize"),
]

# The tree's own green, re-run after every restore.
SANITY = ("kayfabe-tests", "isolate_spawn_is_guest_caused",
          "realizing_a_device_materializes_no_isolate_at_all")


def run(pkg, target, filt):
    cmd = ["cargo", "test", "-p", pkg]
    if target:
        cmd += ["--test", target]
    else:
        cmd += ["--lib"]
    cmd += ["--no-fail-fast", "--", "--exact", filt]
    p = subprocess.run(cmd, cwd=ROOT, env=ENV, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def main():
    results = []
    for name, rel, old, new, pkg, target, filt in BITES:
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        if src.count(old) != 1:
            results.append((name, "★ PATTERN NOT UNIQUE (%d hits) -- bite not applied"
                            % src.count(old)))
            continue
        backup = src
        open(path, "w").write(src.replace(old, new))
        os.utime(path, None)
        time.sleep(0.05)
        rc, out = run(pkg, target, filt)
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
    rc, _ = run(*SANITY)
    print("\n=== LAZY-ISOLATE (E0b) + REFUSAL-VISIBILITY (E1) BITE LEDGER ===")
    for n, r in results:
        print(f"  {r:<50} {n}")
    print(f"\nrestored tree sanity check ({SANITY[2]}): "
          f"{'GREEN' if rc == 0 else 'RED -- RESTORE FAILED'}")
    bad = [n for n, r in results if not r.startswith("BITES")]
    print(f"\n{len(results) - len(bad)}/{len(results)} bites fired")
    sys.exit(1 if bad or rc != 0 else 0)


main()
