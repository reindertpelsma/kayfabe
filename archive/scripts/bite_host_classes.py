#!/usr/bin/env python3
"""Bite harness for the host-forwarding class seam (`#156`).

Plant a wrong class id — or a wrong *role*, or a delegation that silently answers
"no profile" — and require the guard to go RED. A guard nobody has watched fail is
not a guard.

★★ The harness bites THREE layers and reports them separately:

  PROFILE  `crates/kayfabe-chips/src/host_classes.rs` and the three `impl Arch`
           hooks — the numbers and which arch declares which. Watched by
           `crates/kayfabe-chips/tests/host_classes.rs`.

  WIRING   `crates/kayfabe-isolate-host/src/rm.rs` — which ROLE each call site
           asks the profile for.

  TYPING   `crates/kayfabe-arch/src/lib.rs` — the three ways the role TYPES
           themselves can be dismantled in one quiet line (alias two roles, add a
           uniform escape, untag at the call site). Watched by
           `tests/tests/host_class_role_wiring.rs`.

★★★ HISTORY, because the numbers are the point of this file.

  At `36f746a` (#156) this harness measured `PROFILE: 9/9 caught. WIRING: 0/3
  caught.` — every VALUE bite fired and not one ROLE bite did. The three ids were
  all `ClassId`, so a swap was type-correct everywhere, and a real Hopper host
  SERVES two of the three wrong roles (`g_gpu_class_list.c:1996/:1997`): no error,
  no Xid, nothing to notice.

  `#166` made the three roles three distinct TYPES. The WIRING bites now fail to
  COMPILE, which is why `run()` below records `red_kind` and this script prints
  COMPILE vs TEST separately: "the mutated tree does not build" is a strictly
  stronger refusal than "a test went red", and a harness that flattened the two
  would hide which one it got.

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
ARCH_RS = os.path.join(ROOT, "crates/kayfabe-arch/src/lib.rs")

# The guard under test, and a second suite so a bite that trips something else is
# not credited to the guard.
GUARD = ["cargo", "test", "-q", "-p", "kayfabe-chips", "--test", "host_classes"]
INVARIANT_GUARD = ["cargo", "test", "-q", "-p", "kayfabe-abi", "--test", "invariant_classes"]
HOST_UNIT = ["cargo", "test", "-q", "-p", "kayfabe-isolate-host", "--lib"]
# ★ #166's gate: the three ways a type-level refusal gets dismantled in one line.
ROLE_GATE = ["cargo", "test", "-q", "-p", "kayfabe-tests", "--test", "host_class_role_wiring"]

# (layer, name, file, old, new) — `old` must appear EXACTLY ONCE in the file.
BITES = [
    # ── PROFILE: the wrong number ────────────────────────────────────────────
    (
        "PROFILE",
        "the Hopper CE object reverts to the Ampere class (the LOUD mis-route)",
        PROFILE_RS,
        """    fn ce_object(&self) -> CeObjectClass {
        CeObjectClass::new(ClassId(nv::HOPPER_DMA_COPY_A))
    }""",
        """    fn ce_object(&self) -> CeObjectClass {
        CeObjectClass::new(ClassId(nv::AMPERE_DMA_COPY_B))
    }""",
    ),
    (
        "PROFILE",
        "the Hopper CHANNEL reverts to the Ampere class (SILENT on real silicon)",
        PROFILE_RS,
        """    fn gpfifo_channel(&self) -> ChannelClass {
        ChannelClass::new(ClassId(nv::HOPPER_CHANNEL_GPFIFO_A))
    }""",
        """    fn gpfifo_channel(&self) -> ChannelClass {
        ChannelClass::new(ClassId(nv::AMPERE_CHANNEL_GPFIFO_A))
    }""",
    ),
    (
        "PROFILE",
        "the Hopper USERMODE reverts to the Ampere class (SILENT on real silicon)",
        PROFILE_RS,
        """    fn usermode(&self) -> UsermodeClass {
        UsermodeClass::new(ClassId(nv::HOPPER_USERMODE_A))
    }""",
        """    fn usermode(&self) -> UsermodeClass {
        UsermodeClass::new(ClassId(nv::AMPERE_USERMODE_A))
    }""",
    ),
    # ── PROFILE: the wrong ROLE, same generation ─────────────────────────────
    (
        "PROFILE",
        "GA10x wears the usermode ROLE TAG over the channel NUMBER (types cannot see this)",
        PROFILE_RS,
        """    fn usermode(&self) -> UsermodeClass {
        UsermodeClass::new(ClassId(nv::AMPERE_USERMODE_A))
    }
    fn ce_object(&self) -> CeObjectClass {
        CeObjectClass::new(ClassId(nv::AMPERE_DMA_COPY_B))
    }
}

/// The AD10x host-class profile""",
        """    fn usermode(&self) -> UsermodeClass {
        UsermodeClass::new(ClassId(nv::AMPERE_CHANNEL_GPFIFO_A))
    }
    fn ce_object(&self) -> CeObjectClass {
        CeObjectClass::new(ClassId(nv::AMPERE_DMA_COPY_B))
    }
}

/// The AD10x host-class profile""",
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
    #
    # ★ Each of these is now a TYPE ERROR rather than a wrong number. The mutated
    # tree does not compile, so `HOST_UNIT` fails at the `Checking` stage — which
    # is why the harness reports the red_kind.
    (
        "WIRING",
        "the doorbell window is allocated as the CHANNEL class, not the usermode one",
        RM_RS,
        "let usermode = conn.open_usermode(conn.classes.usermode());",
        "let usermode = conn.open_usermode(conn.classes.gpfifo_channel());",
    ),
    (
        "WIRING",
        "the host channel is allocated as the CE-object class",
        RM_RS,
        "            self.conn.classes.gpfifo_channel(),",
        "            self.conn.classes.ce_object(),",
    ),
    (
        "WIRING",
        "SET_OBJECT carries the channel class instead of the CE-object class",
        RM_RS,
        "            class_id: self.conn.classes.ce_object(),",
        "            class_id: self.conn.classes.gpfifo_channel(),",
    ),
    (
        "WIRING",
        "the CE engine object is allocated as the USERMODE class",
        RM_RS,
        "self.alloc_ce_engine_object(chan, self.conn.classes.ce_object(), &params)",
        "self.alloc_ce_engine_object(chan, self.conn.classes.usermode(), &params)",
    ),
    # ── TYPING: the three ways the refusal itself gets dismantled ────────────
    (
        "TYPING",
        "two roles are ALIASED to one type — every swap compiles again",
        ARCH_RS,
        """pub struct UsermodeClass(ClassId);

impl UsermodeClass {
    /// Tag a class id as the usermode role.
    #[must_use]
    pub const fn new(id: ClassId) -> Self {
        Self(id)
    }

    /// Untag — see [`ChannelClass::channel_id`].
    #[must_use]
    pub const fn usermode_id(self) -> ClassId {
        self.0
    }
}""",
        """pub struct UsermodeClassPlaceholder(ClassId);

/// ★ THE BITE: two roles, one type. Everything still compiles; every swap is legal.
pub type UsermodeClass = ChannelClass;

impl ChannelClass {
    /// Untag as the usermode role.
    #[must_use]
    pub const fn usermode_id(self) -> ClassId {
        self.0
    }
}""",
    ),
    (
        "TYPING",
        "a UNIFORM escape appears — ClassId::from(role) untags without naming a role",
        ARCH_RS,
        """impl CeObjectClass {""",
        """impl From<CeObjectClass> for ClassId {
    fn from(v: CeObjectClass) -> Self {
        v.0
    }
}

impl CeObjectClass {""",
    ),
    (
        "TYPING",
        "the host adapter untags AT the call site, rebuilding the bare-u32 hole",
        RM_RS,
        "let usermode = conn.open_usermode(conn.classes.usermode());",
        "let _leak = conn.classes.usermode().usermode_id().0;\n        let usermode = conn.open_usermode(conn.classes.usermode());",
    ),
]


def run(cmd, env):
    return subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True)


def first_error(out, keep=6):
    """The most SPECIFIC diagnostic in `out`, so a RED verdict carries its attribution.

    ★★ A bite result that prints only RED/GREEN is a boolean witness, and a boolean
    witness cannot attribute: it is compatible with the tree failing for a reason that
    has nothing to do with the mutation. This function exists because of a real reading
    (2026-08-01): the first WIRING pass printed "failed to run custom build command …
    exit status: 101" for all four bites — TRUE, USELESS, and equally consistent with any
    breakage at all. The rustc `error[E0308]` that actually names
    "expected `UsermodeClass`, found `ChannelClass`" was thirty lines further down,
    inside the build script's captured stderr (`build.rs` cross-compiles the isolate
    image for musl, so a type error in `rm.rs` surfaces as a build-script panic before it
    surfaces as itself).

    So the search is ordered by SPECIFICITY, not by position: a typed rustc diagnostic
    beats a panic message beats an assertion beats cargo's summary line.

    ⊘ And a second reading, in this same file: the edit that first introduced this
    function spliced it in at the wrong index and left **two** definitions of it — the
    later, weaker one won, so the improved evidence never appeared and the campaign
    printed the useless line again. `suspect_the_instrument_first`: the harness was the
    defect, twice in a row, on the thing whose whole job is to tell me when something is
    wrong.
    """
    lines = out.splitlines()
    for pred in (
        lambda line: line.lstrip().startswith("error[E"),
        lambda line: "panicked at" in line,
        lambda line: line.lstrip().startswith("assertion"),
        lambda line: line.lstrip().startswith("error"),
    ):
        for k, line in enumerate(lines):
            if pred(line):
                return "\n".join(x.rstrip() for x in lines[k : k + keep])
    return "\n".join(lines[-keep:])


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
    guards = [
        ("guard", GUARD),
        ("invariant", INVARIANT_GUARD),
        ("host-unit", HOST_UNIT),
        ("role-gate", ROLE_GATE),
    ]
    base = {n: run(c, env).returncode == 0 for n, c in guards}
    print("baseline: " + " ".join(f"{n}={'GREEN' if ok else 'RED'}" for n, ok in base.items()))
    if not all(base.values()):
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
                results.append((i, layer, name, None, "", ""))
                continue
            with open(path, "w", encoding="utf-8") as f:
                f.write(original.replace(old, new))
            # ★ mtime, ALWAYS. `cp -a`/`shutil.move`/`rsync -a` preserve it and cargo's
            # freshness check is mtime-based, so a restore that keeps the old stamp
            # serves a STALE rlib and manufactures inflated bite sets.
            os.utime(path, None)
            time.sleep(0.01)
            red, kind, evidence = False, "", ""
            for gname, cmd in guards:
                r = run(cmd, env)
                if r.returncode != 0:
                    red = True
                    out = (r.stdout or "") + (r.stderr or "")
                    # ★★ COMPILE vs TEST is the distinction #166 turns on. A mutated
                    # tree that does not BUILD is a strictly stronger refusal than one
                    # whose test went red, and a harness that flattened the two would
                    # not be able to say which it got.
                    kind = "COMPILE" if "error[E0" in out or "error: could not compile" in out else "TEST"
                    evidence = first_error(out)
                    break
            with open(path, "w", encoding="utf-8") as f:
                f.write(original)
            os.utime(path, None)
            tag = f"{kind:7s}" if red else "GREEN  "
            print(f"{i:2d}  [{layer:7s}] {tag}  {name}", flush=True)
            if red and evidence:
                for line in evidence.splitlines():
                    print(f"        | {line}")
            results.append((i, layer, name, red, kind, evidence))
    finally:
        for path, text in originals.items():
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            os.utime(path, None)

    print()
    rc = 0
    for layer in ("PROFILE", "WIRING", "TYPING"):
        rows = [r for r in results if r[1] == layer]
        if not rows:
            continue
        caught = [r for r in rows if r[3]]
        kinds = sorted({r[4] for r in caught})
        print(
            f"{layer:7s}: {len(caught)}/{len(rows)} caught"
            + (f"  ({', '.join(kinds)})" if kinds else "")
        )
        for i, _l, name, red, _k, _e in rows:
            if not red:
                print(f"  ★★ {layer} MISS: {i:2d} {name}")
                rc = 1
    if rc:
        print(
            "\n★★★ A MISS is an UNCOVERED SURFACE. Before writing a test for it, read the "
            "line: a surviving mutant can also mean the code is wrong."
        )
    return rc


if __name__ == "__main__":
    sys.exit(main())
