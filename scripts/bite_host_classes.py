#!/usr/bin/env python3
"""Bite harness for the host-forwarding class seam (`#156`).

Plant a wrong class id — or a wrong *role*, or a delegation that silently answers
"no profile" — and require the guard to go RED. A guard nobody has watched fail is
not a guard.

★★ The harness deliberately bites TWO layers and reports them separately:

  PROFILE  `crates/kayfabe-chips/src/host_classes.rs` and the three `impl Arch`
           hooks — the numbers and which arch declares which. Watched by
           `crates/kayfabe-chips/tests/host_classes.rs`.

  WIRING   `crates/kayfabe-isolate-host/src/rm.rs` — which ROLE each call site
           asks the profile for. This layer runs against a real host GPU and
           nothing in the offline suite reaches it.

⊘ The WIRING arm is EXPECTED TO MISS, and printing that is the point. A bite that
does not fire is an instrument claim until proven otherwise — so the arm shares a
harness with bites that DO fire, which is what distinguishes "no guard exists"
from "the harness is broken". If a WIRING bite ever starts firing, something new
is covering it and this comment is stale.

Usage:
    scripts/bite_host_classes.py [--only N] [--list]

Every bite is applied to the working tree and restored afterwards, and the file's
mtime is touched after restoring — `shutil`/`cp -a` preserve mtime, cargo then
serves a stale rlib, and the next bite is measured against the previous binary.
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

PROFILE_RS = os.path.join(ROOT, "crates/kayfabe-chips/src/host_classes.rs")
GH100_RS = os.path.join(ROOT, "crates/kayfabe-chips/src/gh100.rs")
AD10X_RS = os.path.join(ROOT, "crates/kayfabe-chips/src/ad10x.rs")
INVARIANT_RS = os.path.join(ROOT, "crates/kayfabe-abi/src/invariant_classes.rs")
RM_RS = os.path.join(ROOT, "crates/kayfabe-isolate-host/src/rm.rs")

# The guard under test, and a second suite so a bite that trips something else is
# not credited to the guard.
GUARD = ["cargo", "test", "-q", "-p", "kayfabe-chips", "--test", "host_classes"]
INVARIANT_GUARD = ["cargo", "test", "-q", "-p", "kayfabe-abi", "--test", "invariant_classes"]
HOST_UNIT = ["cargo", "test", "-q", "-p", "kayfabe-isolate-host", "--lib"]

# (layer, name, file, old, new) — `old` must appear EXACTLY ONCE in the file.
BITES = [
    # ── PROFILE: the wrong number ────────────────────────────────────────────
    (
        "PROFILE",
        "the Hopper CE object reverts to the Ampere class (the LOUD mis-route)",
        PROFILE_RS,
        """    fn ce_object(&self) -> ClassId {
        ClassId(nv::HOPPER_DMA_COPY_A)
    }""",
        """    fn ce_object(&self) -> ClassId {
        ClassId(nv::AMPERE_DMA_COPY_B)
    }""",
    ),
    (
        "PROFILE",
        "the Hopper CHANNEL reverts to the Ampere class (SILENT on real silicon)",
        PROFILE_RS,
        """    fn gpfifo_channel(&self) -> ClassId {
        ClassId(nv::HOPPER_CHANNEL_GPFIFO_A)
    }""",
        """    fn gpfifo_channel(&self) -> ClassId {
        ClassId(nv::AMPERE_CHANNEL_GPFIFO_A)
    }""",
    ),
    (
        "PROFILE",
        "the Hopper USERMODE reverts to the Ampere class (SILENT on real silicon)",
        PROFILE_RS,
        """    fn usermode(&self) -> ClassId {
        ClassId(nv::HOPPER_USERMODE_A)
    }""",
        """    fn usermode(&self) -> ClassId {
        ClassId(nv::AMPERE_USERMODE_A)
    }""",
    ),
    # ── PROFILE: the wrong ROLE, same generation ─────────────────────────────
    (
        "PROFILE",
        "GA10x answers the channel class where usermode was asked",
        PROFILE_RS,
        """impl HostClasses for Ga10xHostClasses {
    fn name(&self) -> &'static str {
        "GA10x host classes (GA106)"
    }
    fn gpfifo_channel(&self) -> ClassId {
        ClassId(nv::AMPERE_CHANNEL_GPFIFO_A)
    }
    fn usermode(&self) -> ClassId {
        ClassId(nv::AMPERE_USERMODE_A)
    }""",
        """impl HostClasses for Ga10xHostClasses {
    fn name(&self) -> &'static str {
        "GA10x host classes (GA106)"
    }
    fn gpfifo_channel(&self) -> ClassId {
        ClassId(nv::AMPERE_CHANNEL_GPFIFO_A)
    }
    fn usermode(&self) -> ClassId {
        ClassId(nv::AMPERE_CHANNEL_GPFIFO_A)
    }""",
    ),
    # ── PROFILE: the pin moves off the measured part ─────────────────────────
    (
        "PROFILE",
        "the isolate's pinned profile silently becomes an UNMEASURED generation",
        PROFILE_RS,
        """pub fn pinned_host_classes() -> &'static dyn HostClasses {
    &Ga10xHostClasses
}""",
        """pub fn pinned_host_classes() -> &'static dyn HostClasses {
    &Gh100HostClasses
}""",
    ),
    # ── PROFILE: the delegation trap the seam was built to avoid ─────────────
    (
        "PROFILE",
        "the GH100 arch DELEGATES host_classes to its composed mock (answers None)",
        GH100_RS,
        """    fn host_classes(&self) -> Option<&dyn HostClasses> {
        Some(&crate::host_classes::Gh100HostClasses)
    }""",
        """    fn host_classes(&self) -> Option<&dyn HostClasses> {
        self.inner.host_classes()
    }""",
    ),
    (
        "PROFILE",
        "the AD10x arch hands back the GH100 profile",
        AD10X_RS,
        """    fn host_classes(&self) -> Option<&dyn HostClasses> {
        Some(&crate::host_classes::Ad10xHostClasses)
    }""",
        """    fn host_classes(&self) -> Option<&dyn HostClasses> {
        Some(&crate::host_classes::Gh100HostClasses)
    }""",
    ),
    # ── PROFILE: the ⊘ half — an invariant role rewired ───────────────────────
    (
        "PROFILE",
        "the invariant VA_SPACE role is wired to the channel-group class",
        INVARIANT_RS,
        "pub const VA_SPACE: u32 = classes::FERMI_VASPACE_A;",
        "pub const VA_SPACE: u32 = classes::KEPLER_CHANNEL_GROUP_A;",
    ),
    (
        "PROFILE",
        "a FOURTH class joins the invariant set with no per-chip citation",
        INVARIANT_RS,
        """    ("CONTEXT_SHARE", CONTEXT_SHARE),
];""",
        """    ("CONTEXT_SHARE", CONTEXT_SHARE),
    ("VA_SPACE", VA_SPACE),
];""",
    ),
    # ── WIRING: the role each rm.rs call site asks for ───────────────────────
    (
        "WIRING",
        "the doorbell window is allocated as the CHANNEL class, not the usermode one",
        RM_RS,
        "self.raw_alloc(self.subdevice, want, self.classes.usermode().0, &mut [])?;",
        "self.raw_alloc(self.subdevice, want, self.classes.gpfifo_channel().0, &mut [])?;",
    ),
    (
        "WIRING",
        "the host channel is allocated as the CE-object class",
        RM_RS,
        "            self.conn.classes.gpfifo_channel().0,",
        "            self.conn.classes.ce_object().0,",
    ),
    (
        "WIRING",
        "SET_OBJECT carries the channel class instead of the CE-object class",
        RM_RS,
        "            class_id: self.conn.classes.ce_object().0,",
        "            class_id: self.conn.classes.gpfifo_channel().0,",
    ),
]


def run(cmd, env):
    return subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", type=int)
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    if args.list:
        for i, (layer, name, *_rest) in enumerate(BITES, 1):
            print(f"{i:2d}  [{layer}] {name}")
        return 0

    env = dict(os.environ)
    env.setdefault("KAYFABE_NO_KVM", "1")

    originals = {}
    for _layer, _name, path, _old, _new in BITES:
        if path not in originals:
            with open(path, encoding="utf-8") as f:
                originals[path] = f.read()

    # ★ The baseline. Without it a harness whose guard is ALREADY red reports every
    # bite as caught, which is the flattering failure.
    base_guard = run(GUARD, env).returncode == 0
    base_inv = run(INVARIANT_GUARD, env).returncode == 0
    base_unit = run(HOST_UNIT, env).returncode == 0
    print(
        f"baseline: guard={'GREEN' if base_guard else 'RED'} "
        f"invariant={'GREEN' if base_inv else 'RED'} "
        f"host-unit={'GREEN' if base_unit else 'RED'}"
    )
    if not (base_guard and base_inv and base_unit):
        print("★★ BASELINE IS NOT GREEN — every result below would be meaningless.")
        return 2

    results = []
    try:
        for i, (layer, name, path, old, new) in enumerate(BITES, 1):
            if args.only and i != args.only:
                continue
            original = originals[path]
            n = original.count(old)
            if n != 1:
                print(f"{i:2d}  [{layer}] {name}\n    ★ ANCHOR MATCHED {n} TIMES — not applied")
                results.append((i, layer, name, None))
                continue
            with open(path, "w", encoding="utf-8") as f:
                f.write(original.replace(old, new))
            os.utime(path, None)
            time.sleep(0.01)
            red = (
                run(GUARD, env).returncode != 0
                or run(INVARIANT_GUARD, env).returncode != 0
                or run(HOST_UNIT, env).returncode != 0
            )
            with open(path, "w", encoding="utf-8") as f:
                f.write(original)
            os.utime(path, None)
            print(f"{i:2d}  [{layer}] {'RED  ' if red else 'GREEN'}  {name}", flush=True)
            results.append((i, layer, name, red))
    finally:
        for path, text in originals.items():
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            os.utime(path, None)

    prof = [r for r in results if r[1] == "PROFILE"]
    wire = [r for r in results if r[1] == "WIRING"]
    prof_caught = [r for r in prof if r[3]]
    wire_caught = [r for r in wire if r[3]]
    print(
        f"\nPROFILE: {len(prof_caught)}/{len(prof)} caught."
        f"  WIRING: {len(wire_caught)}/{len(wire)} caught."
    )
    if prof and len(prof_caught) < len(prof):
        for i, _l, name, red in prof:
            if not red:
                print(f"  ★★ PROFILE MISS: {i:2d} {name}")
    if wire and not wire_caught:
        print(
            "  ⊘ EXPECTED: no offline test reaches rm.rs's role wiring — that path "
            "needs a host GPU. The PROFILE arm above is the proof this harness works, "
            "so these are an UNCOVERED SURFACE and not a broken instrument."
        )
    # Only a PROFILE miss is a failure; the WIRING arm is a measurement.
    return 1 if prof and len(prof_caught) < len(prof) else 0


if __name__ == "__main__":
    sys.exit(main())
