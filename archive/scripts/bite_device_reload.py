#!/usr/bin/env python3
"""The `#130` unload -> reload bite harness — remove each guarantee, WATCH it go red, restore.

`#130` is the owner's requirement that a guest which bricks the emulator can be recovered by
unloading and reloading the device, the way a real card recovers from `rmmod nvidia;
modprobe nvidia`. What it asserts is an ABSENCE — no residual state, no surviving isolate —
and an absence is the easiest thing in the world to assert vacuously. So every guarantee in
that work is un-done here, compiled, and run.

WHY IT IS COMMITTED RATHER THAN RUN ONCE. A bite ledger in a commit message is a claim about
a tree that has since moved. This is re-runnable at any revision, which is the difference
between "eleven bites fired once" and "eleven bites fire".

TWO PHASES, because the guarantees are of two kinds.

  ★★★ PHASE 1 — THE STRUCTURAL ONES, where the bite is a COMPILE ERROR.

  "No residual state across a reload" is quantified over every field of the device, and this
  repository's most-repeated defect is a gate quantified over a hand-written list: shortening
  the list weakens the gate with zero red tests. So the projections that carry device state
  outward (`RegPlane::counters`, `RegPlane::residue`, `Regs::audit`, `Shim::audit`)
  DESTRUCTURE their source with no `..`. Each bite here adds one field to a state-holding
  struct — and fills every constructor of it, so the only thing that can still be red is the
  projection — and requires `error[E0027]` at the projection's own file. A structural
  guarantee whose failure mode nobody has seen is a guarantee nobody has checked.

  ★★★ PHASE 2 — THE BEHAVIOURAL ONES, where the bite is a red test.

  Each plants one realistic defect and names the tests that MUST go red. Extra reds are
  reported, not required: a defect commonly trips a precondition somewhere else first, and
  demanding an exact set makes the harness fragile about things it is not measuring. What it
  refuses to accept is a bite that reddens NOTHING.

★ THREE FAILURE MODES IT REPORTS RATHER THAN HIDES, because each looks like success from a
distance:
  - PATTERN NOT UNIQUE — the code moved or was reformatted, so the bite was never applied
    and the green says nothing. NOT a pass; it aborts.
  - DID NOT COMPILE (phase 2) — the defect was rejected by the compiler rather than by the
    test. Inconclusive: the test never ran, and it is reported as a red target.
  - NON-BITER — everything stayed green without the guarantee. That is the finding this
    exists for.

★★ Every file is rewritten (never copied back) on both plant and restore, so its mtime is
bumped. `cp -a` and `shutil.move` preserve mtimes and cargo then serves a stale rlib, which
manufactures false non-biters and has bitten this project before.

Usage:  python3 scripts/bite_device_reload.py [bite-name]   (from anywhere)
Exit:   0 if every bite fired, 1 otherwise.
"""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

PLANE = ROOT / "crates/kayfabe-device/src/plane.rs"
SHIM = ROOT / "crates/kayfabe-qemu-raw/src/shim.rs"
VMMQ = ROOT / "crates/kayfabe-vmm-qemu/src/lib.rs"
MOCKS = ROOT / "crates/kayfabe-mocks/src/lib.rs"
GPU = ROOT / "crates/kayfabe-core/src/gpu.rs"
RECYCLE = ROOT / "crates/kayfabe-qemu-raw/tests/device_recycle.rs"

TOUCHED = (PLANE, SHIM, VMMQ, MOCKS, GPU, RECYCLE)

# The two test targets #130 owns. Both are run for every phase-2 bite, because the whole
# point of the pairing is that one seam's defect must not be silently absorbed by the other.
TARGETS = [
    ("kayfabe-tests", "device_reload_isolates"),
    ("kayfabe-qemu-raw", "device_recycle"),
]


def sub(path, old, new, n=1):
    t = path.read_text()
    got = t.count(old)
    if got != n:
        raise SystemExit(
            f"PATTERN NOT UNIQUE in {path.relative_to(ROOT)}: {got} hits (want {n}) for "
            f"{old[:80]!r}\n★ The bite was never applied. Fix the anchor; a green run here "
            f"means nothing."
        )
    path.write_text(t.replace(old, new))


# =====================================================================================
# PHASE 1 — the structural guarantees: a new state field must not compile
# =====================================================================================

STRUCTURAL = {
    # A new atomic on the register plane's counter block -> `RegPlane::counters` refuses.
    "plane_counters": (
        [(PLANE, "struct PlaneCounters {", "struct PlaneCounters {\n    bite_probe: AtomicU64,")],
        "crates/kayfabe-device/src/plane.rs",
    ),
    # A new mutable field behind the plane's lock -> `RegPlane::residue` refuses.
    "plane_state": (
        [
            (PLANE, "struct PlaneState {", "struct PlaneState {\n    bite_probe: Vec<u64>,"),
            (PLANE, "                unclaimed: Vec::new(),",
             "                bite_probe: Vec::new(),\n                unclaimed: Vec::new(),"),
        ],
        "crates/kayfabe-device/src/plane.rs",
    ),
    # A new field on the register plane itself -> `RegPlane::residue` refuses.
    "reg_plane": (
        [
            (PLANE, "pub struct RegPlane {", "pub struct RegPlane {\n    bite_probe: u64,"),
            (PLANE, "        Ok(RegPlane {\n            chip,",
             "        Ok(RegPlane {\n            bite_probe: 0,\n            chip,"),
        ],
        "crates/kayfabe-device/src/plane.rs",
    ),
    # A new counter on the device-side snapshot -> the WIRE projection `Regs::audit` refuses.
    "device_counters": (
        [
            (PLANE, "pub struct Counters {", "pub struct Counters {\n    pub bite_probe: u64,"),
            (PLANE, "        Counters {\n            reads: g(reads),",
             "        Counters {\n            bite_probe: 0,\n            reads: g(reads),"),
        ],
        "crates/kayfabe-qemu-raw/src/shim.rs",
    ),
    # A new counter on the memory plane's report -> the WIRE projection `Shim::audit` refuses.
    "audit_report": (
        [
            (VMMQ, "pub struct AuditReport {", "pub struct AuditReport {\n    pub bite_probe: u64,"),
            (VMMQ, "        AuditReport {", "        AuditReport {\n            bite_probe: 0,"),
        ],
        "crates/kayfabe-qemu-raw/src/shim.rs",
    ),
}


def build():
    p = subprocess.run(
        ["cargo", "build", "-p", "kayfabe-device", "-p", "kayfabe-qemu-raw",
         "--message-format=short"],
        cwd=ROOT, capture_output=True, text=True,
    )
    return p.returncode, p.stdout + p.stderr


# =====================================================================================
# PHASE 2 — the behavioural guarantees: a red test
# =====================================================================================

BEHAVIOURAL = {
    # The instrument itself. Without a death witness the isolate property is unfalsifiable,
    # and an unfalsifiable property reads exactly like one that holds.
    "no_death_witness": (
        lambda: sub(
            MOCKS,
            "        let mut r = self.recorder.lock().unwrap_or_else(|e| e.into_inner());\n"
            "        r.isolates_dropped.push(self.id);",
            "        let _ = &self.recorder;",
        ),
        {"the_death_witness_is_silent_until_something_actually_dies",
         "unloading_the_device_kills_every_isolate_including_the_unreachable_ones",
         "the_reload_a_retire_would_give_you_is_not_a_reload",
         "a_reloaded_device_is_a_first_boot_and_the_bricked_life_is_gone"},
    ),
    # A teardown that loses exactly the procs ordinary reclamation could not reach — the
    # wedged ones. This is the shape #130 exists for, and it is invisible to a happy path.
    "retired_proc_leaks_its_isolates": (
        lambda: sub(
            GPU,
            "    fn drop(&mut self) {\n"
            "        if std::thread::panicking() || self.pending_release.is_empty() {",
            "    fn drop(&mut self) {\n        if self.is_retired() {\n"
            "            core::mem::forget(core::mem::take(&mut self.isolates));\n        }\n"
            "        if std::thread::panicking() || self.pending_release.is_empty() {",
        ),
        {"unloading_the_device_kills_every_isolate_including_the_unreachable_ones",
         "a_reloaded_device_is_a_first_boot_and_the_bricked_life_is_gone",
         "the_reload_a_retire_would_give_you_is_not_a_reload"},
    ),
    # The other direction: `retire` is pinned as NOT a kill. If someone makes it one, the
    # test that documents the difference must notice — a doc comment does not go red.
    "retire_starts_killing": (
        lambda: sub(
            GPU,
            "    pub fn retire(&mut self) -> Cancels {\n"
            "        // Order matters and is not stylistic",
            "    pub fn retire(&mut self) -> Cancels {\n        self.isolates.clear();\n"
            "        // Order matters and is not stylistic",
        ),
        {"the_reload_a_retire_would_give_you_is_not_a_reload",
         "unloading_the_device_kills_every_isolate_including_the_unreachable_ones"},
    ),
    # The residue members `#130` added must be load-bearing, not decoration.
    "residue_drops_fb_window": (
        lambda: sub(PLANE, "            fb_window: fb_window.clone(),",
                    "            fb_window: Vec::new(),"),
        {"the_dirty_state_is_visible_through_the_comparison_the_property_uses"},
    ),
    "residue_drops_the_gsp": (
        lambda: sub(PLANE, "            gsp: fsm.clone(),",
                    "            gsp: { let mut g = fsm.clone(); g.device_reset(); g },"),
        # ★ MEASURED, and the prediction here was WRONG the first time:
        # `every_boot_point_…_distinct_state` stays GREEN, because the route counters alone
        # already separate all nine points. The emulated GSP's presence in the residue is
        # load-bearing for the two below and not for that one — the sort of thing only a
        # bite tells you.
        {"the_dirty_state_is_visible_through_the_comparison_the_property_uses",
         "a_reloaded_device_is_indistinguishable_from_a_first_boot"},
    ),
    # The 2026-07-31 reference leak, replanted: `unrealize` stops giving the machine its
    # region references back. Here to prove the NEW per-boot-point cycle catches it too, and
    # not only the three-lives test that originally found it.
    "unrealize_stops_draining_references": (
        lambda: sub(
            VMMQ,
            "        for mr in orphaned {\n            p.host.unref_region(mr);\n        }",
            "        drop(orphaned);",
        ),
        {"the_machine_a_reloaded_device_leaves_behind_is_the_machine_it_found",
         "a_reload_from_every_point_of_the_boot_is_a_first_boot"},
    ),
}


def run_tests():
    """{test name: 'ok' | 'failed'} across both targets; a target that will not build is
    reported as one synthetic red row rather than as silence."""
    res = {}
    for pkg, test in TARGETS:
        p = subprocess.run(
            ["cargo", "test", "--no-fail-fast", "-p", pkg, "--test", test,
             "--", "--format", "json", "-Zunstable-options"],
            cwd=ROOT, env={**_env(), "RUSTC_BOOTSTRAP": "1"},
            capture_output=True, text=True,
        )
        found = False
        for line in p.stdout.splitlines():
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                j = json.loads(line)
            except ValueError:
                continue
            if j.get("type") == "test" and j.get("event") in ("ok", "failed"):
                res[j["name"]] = j["event"]
                found = True
        if not found:
            res[f"<{pkg}/{test} DID NOT BUILD OR RAN NOTHING>"] = "failed"
    return res


def _env():
    import os
    return dict(os.environ)


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else None
    failures = []

    # ---- phase 1 ----------------------------------------------------------------
    rc, out = build()
    if rc != 0:
        print("BASELINE IS RED — cannot bite:\n" + out[-3000:])
        return 1
    print("=== PHASE 1: the structural guarantees (a new field must not compile) ===")
    for name, (edits, want_file) in STRUCTURAL.items():
        if only and only != name:
            continue
        saved = {p: p.read_text() for p, _, _ in edits}
        try:
            for path, old, new in edits:
                sub(path, old, new)
            rc, out = build()
            errs = [l for l in out.splitlines() if ": error" in l]
            hit = [l for l in errs if want_file in l]
            ok = rc != 0 and bool(hit)
            print(f"--- {name}: {'FIRED' if ok else 'DID NOT FIRE'}")
            for l in errs[:4]:
                print("    " + l)
            if not ok:
                failures.append(name)
        finally:
            for p, t in saved.items():
                p.write_text(t)

    # ---- phase 2 ----------------------------------------------------------------
    print("\n=== PHASE 2: the behavioural guarantees (a red test) ===")
    base = run_tests()
    reds = [k for k, v in base.items() if v != "ok"]
    if reds:
        print(f"BASELINE IS RED — cannot bite: {reds}")
        return 1
    print(f"baseline: {len(base)} tests, all green")
    for name, (plant, must_red) in BEHAVIOURAL.items():
        if only and only != name:
            continue
        saved = {p: p.read_text() for p in TOUCHED}
        try:
            plant()
            got = {k for k, v in run_tests().items() if v != "ok"}
            ok = bool(got) and must_red.issubset(got)
            print(f"--- {name}: {'FIRED' if ok else 'DID NOT FIRE AS REQUIRED'}")
            print(f"    must be red: {sorted(must_red)}")
            print(f"    all red:     {sorted(got)}")
            if not ok:
                failures.append(name)
        finally:
            for p, t in saved.items():
                p.write_text(t)

    after = run_tests()
    still = [k for k, v in after.items() if v != "ok"]
    print(f"\nrestored: {len(after)} tests, red: {still}")
    if still:
        failures.append("<restore left the tree red>")
    total = len(STRUCTURAL) + len(BEHAVIOURAL) if not only else 1
    print(f"\n{total - len(failures)}/{total} bites fired")
    if failures:
        print("DID NOT FIRE: " + ", ".join(failures))
        return 1
    return 0


sys.exit(main())
