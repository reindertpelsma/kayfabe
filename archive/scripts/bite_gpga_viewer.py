#!/usr/bin/env python3
"""Non-vacuity harness for the GPGA viewer index (`kayfabe_mmu::gpga`).

Each POISON is a one-edit defect in the PRODUCTION file. For each: apply, run the suite
with `--no-fail-fast`, record which tests went red and the first assertion message, then
restore and `touch` (a restored file with an old mtime serves a stale rlib and
manufactures a false non-biter).

A poison that bites nothing is reported as ★ NON-BITER — a test that is green while
measuring nothing.
"""

import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path("/workspace/kf-gpga-viewer")
SRC = ROOT / "crates/kayfabe-mmu/src/gpga.rs"
TEST = "gpga_viewer_index"
# The observer's own floor: fewer than this ran, and the harness — not the code — is what
# is wrong. Every poison here is expected to leave the OTHER tests runnable.
EXPECTED_TESTS = 12

# (name, claim, old, new)
POISONS = [
    (
        "P1-fanout-first-viewer-only",
        "T1: the change reaches ALL viewers, not just one",
        "        for s in self.viewers_of(region) {\n            let occupant = Occupant {",
        "        for s in self.viewers_of(region).into_iter().take(1) {\n            let occupant = Occupant {",
    ),
    (
        "P2-fanout-forgets-the-view-offset",
        "T1: each viewer is told at ITS OWN offset",
        "                .push(ViewUpdate::Shows {\n                    view_off: s.view_off,",
        "                .push(ViewUpdate::Shows {\n                    view_off: 0,",
    ),
    (
        "P3-new-view-is-not-seeded",
        "T2: a new view gets the objects already under it",
        "        let seed = self.contents(region);\n        let seed = self.vouch(viewer, kind, &seed)?;",
        "        let seed = self.contents(region);\n        let mut seed = self.vouch(viewer, kind, &seed)?;\n        seed.clear();",
    ),
    (
        "P4-overflow-drops-silently-without-naming-it",
        "T3: a hanging viewer's lost updates are NAMED (Desynced), not silently dropped",
        "    if v.pending.len() >= MAX_PENDING_UPDATES {\n        v.state = ViewState::Desynced;\n        return;\n    }",
        "    if v.pending.len() >= MAX_PENDING_UPDATES {\n        return;\n    }",
    ),
    (
        "P5-apply-does-not-revalidate-the-plan",
        "T4: R5 — a plan built against an older index is refused (PlanStale)",
        "        if plan.planned_at != self.generation {",
        "        if false && plan.planned_at != self.generation {",
    ),
    (
        "P6-every-view-is-Whole",
        "T5: partial coverage is a Slice, not a Whole",
        "    HostSlice::new(run.base.wrapping_sub(object.base), run.len)\n        .map_or(HostExtent::Whole, HostExtent::Slice)",
        "    HostExtent::Whole",
    ),
    (
        "P7-offset-zero-means-Whole",
        "T5: the regime is 'is this the whole object', NOT 'is the offset zero'",
        "    if run.base == object.base && run.len == object.len {\n        return HostExtent::Whole;\n    }",
        "    if run.base == object.base {\n        return HostExtent::Whole;\n    }",
    ),
    (
        "P8-bare-address-key-drops-the-aperture",
        "T6 / correction 1: the key is (Aperture, address), never a bare address",
        "        let objs = self.objects.get(&region.aperture);\n        let unwit = self.unwitnessed.get(&region.aperture);",
        "        let objs = self.objects.values().next();\n        let unwit = self.unwitnessed.values().next();",
    ),
    (
        "P9-the-dual-never-refuses",
        "T7: an isolate view must not see another isolate's object",
        "        if occupant.owner == viewer_owner {\n            return Ok(());\n        }",
        "        if true || occupant.owner == viewer_owner {\n            return Ok(());\n        }",
    ),
    (
        "P10-free-does-not-retire-the-object",
        "T8: a free removes the object from the INDEX, so no view can still resolve it",
        "    fn retire(&mut self, id: ObjectId) -> Option<ObjectEntry> {\n        for m in self.objects.values_mut() {",
        "    fn retire(&mut self, id: ObjectId) -> Option<ObjectEntry> {\n        if true { return self.object(id).ok(); }\n        for m in self.objects.values_mut() {",
    ),
    (
        "P11-object-ids-are-reused",
        "T8: an ObjectId is never reused, so a stale queued id cannot name a new object",
        "                let id = ObjectId(self.next_object);\n                self.next_object += 1;",
        "                let id = ObjectId(self.next_object);",
    ),
    (
        "P12-unwitnessed-content-is-served-anyway",
        "Correction 3: a view that could be stale REFUSES by name",
        "                    if let Witness::Unwitnessed(transport) = span.witness {\n                        return Err(ViewFault::UnwitnessedContent {\n                            region: span.region,\n                            transport,\n                        });\n                    }",
        "",
    ),
    (
        "P13-the-partition-drops-its-holes",
        "The governing rule: a region query is a TOTAL partition",
        "            out.push(GpgaSpan {\n                region: run,\n                occupant,\n                witness,\n            });",
        "            if occupant.is_none() { continue; }\n            out.push(GpgaSpan {\n                region: run,\n                occupant,\n                witness,\n            });",
    ),
]


def run_suite() -> tuple[list[str], dict[str, str]]:
    """Returns (failed test names, first assertion message per test)."""
    # ⚠ `--no-fail-fast` is a CARGO flag. Putting it after `--` hands it to libtest,
    # which refuses the option, runs NOTHING, and reports zero failures — the first
    # version of this harness did exactly that and scored 0/13 against a green baseline.
    p = subprocess.run(
        ["cargo", "test", "--no-fail-fast", "-p", "kayfabe-tests", "--test", TEST],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    out = p.stdout + p.stderr
    if "error[E" in out or "error: could not compile" in out:
        return ["<COMPILE ERROR>"], {"<COMPILE ERROR>": _first_error(out)}
    # ★★★ SUSPECT THE INSTRUMENT FIRST. A run that executed no test is not a run with no
    # failures. Prove the observer worked before believing what it reports.
    m = re.search(r"test result: \w+\. (\d+) passed; (\d+) failed", out)
    if not m:
        raise SystemExit(f"the suite did not report a result — the harness is broken:\n{out[-2000:]}")
    ran = int(m.group(1)) + int(m.group(2))
    if ran < EXPECTED_TESTS:
        raise SystemExit(f"only {ran} tests ran, expected >= {EXPECTED_TESTS} — the harness is broken")
    failed = re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)
    msgs: dict[str, str] = {}
    for block in out.split("---- ")[1:]:
        name = block.split(" stdout ----")[0].strip()
        body = block.split("\n", 1)[1] if "\n" in block else ""
        m = re.search(r"panicked at [^\n]*\n(.*?)(?:\nnote:|\nstack|\n\n|\Z)", body, re.S)
        if m:
            msgs[name] = m.group(1).strip()
    return failed, msgs


def _first_error(out: str) -> str:
    for line in out.splitlines():
        if line.startswith("error"):
            return line
    return "compile error"


def main() -> int:
    original = SRC.read_text()
    results = []
    try:
        for name, claim, old, new in POISONS:
            if original.count(old) != 1:
                print(f"!! {name}: anchor matched {original.count(old)} times — FIX THE HARNESS")
                results.append((name, claim, None, None))
                continue
            SRC.write_text(original.replace(old, new, 1))
            SRC.touch()
            time.sleep(0.05)
            failed, msgs = run_suite()
            results.append((name, claim, failed, msgs))
            print(f"\n{'=' * 78}\n{name}\n  claim: {claim}")
            if not failed:
                print("  ★★★ NON-BITER — every test stayed GREEN. The claim is not measured.")
            else:
                for f in failed:
                    print(f"  BIT: {f}")
                    if f in msgs:
                        for line in msgs[f].splitlines():
                            print(f"       | {line}")
    finally:
        SRC.write_text(original)
        SRC.touch()
        time.sleep(0.05)
        print(f"\n{'=' * 78}\nRESTORED. Verifying green...")
        failed, _ = run_suite()
        print("baseline:", "GREEN" if not failed else f"STILL RED: {failed}")

    non_biters = [n for n, _, f, _ in results if not f]
    print(f"\n{len(POISONS) - len(non_biters)}/{len(POISONS)} poisons bit.")
    if non_biters:
        print("★ NON-BITERS:", ", ".join(non_biters))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
