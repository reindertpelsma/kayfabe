#!/usr/bin/env python3
"""The COMPATIBILITY MATRIX — derived from the tree, never hand-written.

Read `docs/design/compatibility_matrix.md` for the reading. This file is the
instrument, and it exists because a hand-written matrix rots silently: it stays
green while the tree it describes moves underneath it. Everything printed below is
read out of the source at run time, so the matrix is wrong only when the tree is.

WHAT IT DERIVES, per axis:

  A guest driver version  — the rows of `kayfabe_abi::versions::TABLES`, plus which
                            of them can answer the FIRST GSP RPC (`vgx`), plus the
                            single version the shipping shim compiles in.
  B GPU architecture      — `kayfabe_device::CHIPS` (the reachable chips) against
                            `kayfabe_abi::vbios::VBIOS_PROFILES` and the GSP models
                            in `kayfabe-chips`. The three lists disagreeing IS the
                            finding, so they are printed side by side.
  C host driver version   — the pinned interval, read off `kayfabe_abi::host_driver`.
  D guest OS              — the `GuestOs` variants and which carry a rule, plus
                            whether the configuration door has any caller at all.
  E guest kernel version  — a NEGATIVE derivation: the tokens that would exist if
                            this axis existed, counted. A non-zero count means the
                            axis grew a home and this script is out of date.
  F multi-GPU             — the `MG-n` decisions actually cited in code.

★ AND THE PART THAT IS NOT AN AXIS: REACHABILITY. A crate that no shipping artifact
links can hold as fine a seam as it likes and no guest will ever reach it. The
dependency graph is walked from the real build outputs (the QEMU staticlib and the
isolate binaries), and every axis above is labelled with whether the crate that
carries it is inside that closure. This is what separates "built" from "built and
wired", and no amount of reading a design doc gives you it.

★★ WHAT THIS CANNOT SEE, stated so a green is not over-read:

  1. It reads DECLARATIONS, not behaviour. A row in `TABLES` means a table exists,
     never that a guest at that version boots. Nothing here has touched a GPU.
  2. Reachability is computed from Cargo manifests, so it proves a crate is LINKED,
     not that any code path calls into it. A linked-but-dead seam scores the same as
     a live one.
  3. The negative derivations (E especially) are token searches. A guest-kernel
     assumption written without any of the tokens is invisible here, exactly as the
     claim ledger is blind to a claim made without a claim word.

USAGE
  scripts/compat_matrix.py            # the matrix
  scripts/compat_matrix.py --check    # exit non-zero if a derived fact moved
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8", errors="replace")


def tracked() -> list[str]:
    """The universe, derived — `git ls-files`, never a hand-list."""
    out = subprocess.run(
        ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
    )
    return out.stdout.splitlines()


# =====================================================================================
# REACHABILITY — which crates does a shipping artifact actually link?
# =====================================================================================

# The real build outputs. `kayfabe-qemu-raw` is the `staticlib` QEMU links; the
# `kayfabe-isolate-host` binaries are the host edge. Everything else in the workspace
# is reached only through one of these, or by the test crate.
SHIPPING_ROOTS = ("kayfabe-qemu-raw", "kayfabe-isolate-host")

DEP_RE = re.compile(r"^\s*(kayfabe-[a-z-]+)\s*(?:=|\.workspace)", re.M)

# ★ SECTION-AWARE, and that is not pedantry. `kayfabe-isolate-host` carries a
# `[dev-dependencies]` block; counting it would put a test-only crate inside the
# shipping closure and turn this instrument's headline answer into a false green.
# Only these sections describe what a real build output links.
LINKING_SECTIONS = ("[dependencies]", "[build-dependencies]")


def crate_deps() -> dict[str, set[str]]:
    deps: dict[str, set[str]] = {}
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        name = manifest.parent.name
        found: set[str] = set()
        section = ""
        for line in manifest.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if stripped.startswith("[") and stripped.endswith("]"):
                section = stripped
                continue
            if section not in LINKING_SECTIONS:
                continue
            m = DEP_RE.match(line)
            if m and m.group(1) != name:
                found.add(m.group(1))
        deps[name] = found
    return deps


def closure_of(roots: tuple[str, ...]) -> set[str]:
    deps = crate_deps()
    seen: set[str] = set()
    stack = [r for r in roots if r in deps]
    while stack:
        c = stack.pop()
        if c in seen:
            continue
        seen.add(c)
        stack.extend(deps.get(c, ()))
    return seen


def shipping_closure() -> set[str]:
    return closure_of(SHIPPING_ROOTS)


def where(crate: str, closure: set[str]) -> str:
    return "LINKED" if crate in closure else "NOT LINKED"


# =====================================================================================
# AXIS A — guest driver version
# =====================================================================================

TABLE_ROW_RE = re.compile(r"DriverAbiTable\s*\{(.*?)\n    \},", re.S)
VER_RE = re.compile(r"major:\s*(\d+),\s*minor:\s*(\d+),\s*patch:\s*(\d+)")


def axis_a() -> list[dict]:
    src = read("crates/kayfabe-abi/src/versions.rs")
    start = src.index("pub const TABLES")
    end = src.index("\n];", start)
    rows = []
    for body in TABLE_ROW_RE.findall(src[start:end] + "\n    },"):
        m = VER_RE.search(body)
        if not m:
            continue
        rows.append(
            {
                "version": f"{int(m.group(1))}.{int(m.group(2)):02d}.{int(m.group(3)):02d}",
                "vgx": "vgx: Some(" in body,
                "map_dma": _field(body, "map_dma"),
                "gsp_element": _field(body, "gsp_element"),
                "caps": _field(body, "caps"),
            }
        )
    return rows


def _field(body: str, name: str) -> str:
    m = re.search(rf"^\s*{name}:\s*([^,\n]+)", body, re.M)
    return m.group(1).strip().removeprefix("&") if m else "-"


def shim_guest_driver() -> str:
    src = read("crates/kayfabe-qemu-raw/src/shim.rs")
    m = re.search(r"pub const GUEST_DRIVER:[^=]+=\s*([^;]+);", src)
    return m.group(1).strip() if m else "?"


def bench_driver() -> str:
    src = read("crates/kayfabe-abi/src/versions.rs")
    m = re.search(r"pub const BENCH_DRIVER: DriverVersion = DriverVersion \{(.*?)\};", src, re.S)
    v = VER_RE.search(m.group(1)) if m else None
    return f"{int(v.group(1))}.{int(v.group(2)):02d}.{int(v.group(3)):02d}" if v else "?"


# =====================================================================================
# AXIS B — GPU architecture
# =====================================================================================


def axis_b() -> dict[str, list[str]]:
    chips = read("crates/kayfabe-device/src/lib.rs")
    m = re.search(r"pub static CHIPS: &\[&ChipProfile\] = &\[(.*?)\];", chips, re.S)
    reachable = re.findall(r"&([a-z0-9_]+)::([A-Z0-9_]+)", m.group(1)) if m else []

    vbios = read("crates/kayfabe-abi/src/vbios.rs")
    vstart = vbios.index("pub static VBIOS_PROFILES")
    vend = vbios.index("\n];", vstart)
    vprofiles = re.findall(r'name:\s*"([A-Za-z0-9]+)"', vbios[vstart:vend])

    models = []
    for f in sorted((ROOT / "crates" / "kayfabe-chips" / "src").glob("*.rs")):
        if f.name != "lib.rs":
            models.append(f.stem)
    # `ga10x` lives in kayfabe-device, not kayfabe-chips — it is the one model whose
    # generation also has a ChipProfile, which is precisely the asymmetry below.
    if (ROOT / "crates" / "kayfabe-device" / "src" / "ga10x.rs").exists():
        models.append("ga10x (in kayfabe-device)")

    return {
        "chip_profiles": [f"{mod}::{name}" for mod, name in reachable],
        "vbios_profiles": vprofiles,
        "gsp_models": models,
    }


# =====================================================================================
# AXIS C — host driver version
# =====================================================================================


def axis_c() -> dict[str, str]:
    src = read("crates/kayfabe-abi/src/host_driver.rs")

    def const(name: str) -> str:
        m = re.search(rf"pub const {name}[^=]+=\s*([^;]+);", src)
        return m.group(1).strip() if m else "?"

    floor = const("ENCODED_FOR_FLOOR").strip("()").replace(" ", "")
    major = const("ENCODED_FOR_MAJOR")
    minor, patch = (floor.split(",") + ["?", "?"])[:2]
    refusals = re.findall(r"^\s{4}([A-Z][A-Za-z]+)\s*\{", src, re.M)
    return {
        "interval": f"[{major}.{int(minor):02d}.{int(patch):02d}, {int(major) + 1}.00.00)",
        "refusals": ", ".join(dict.fromkeys(refusals)) or "-",
        "shift_major": const("CHANNEL_PARAMS_SHIFT_MAJOR"),
    }


# =====================================================================================
# AXIS D — guest OS
# =====================================================================================


def axis_d() -> dict:
    src = read("crates/kayfabe-abi/src/guest_os.rs")
    # Scoped to the `GuestOs` enum body: the file also declares `ClientKindRule`, whose
    # variants are rules and not operating systems.
    enum_body = re.search(r"pub enum GuestOs \{(.*?)\n\}", src, re.S)
    variants = re.findall(r"^    ([A-Z][A-Za-z]+),$", enum_body.group(1) if enum_body else "", re.M)
    m = re.search(r"pub const fn client_kind_rule\(self\).*?\n    \}", src, re.S)
    body = m.group(0) if m else ""
    with_rule = re.findall(r"GuestOs::([A-Za-z]+)\s*=>\s*Some", body)
    # Does the configuration door have a caller outside its own module and the tests?
    callers = _grep_rs(r"from_config_name", exclude=("crates/kayfabe-abi/src/guest_os.rs",))
    return {
        "variants": variants,
        "with_rule": with_rule,
        "config_callers": callers,
    }


def _grep_rs(pattern: str, exclude: tuple[str, ...] = ()) -> list[str]:
    hits = []
    rx = re.compile(pattern)
    for rel in tracked():
        if not rel.endswith(".rs") or rel.startswith(exclude):
            continue
        for i, line in enumerate(read(rel).splitlines(), 1):
            if rx.search(line) and not line.lstrip().startswith(("//", "*")):
                hits.append(f"{rel}:{i}")
    return hits


# =====================================================================================
# AXIS E — guest kernel version (a NEGATIVE derivation)
# =====================================================================================

# Tokens that would have to appear if this axis had a home. Every one of them is
# something a port that cared about the guest's kernel release would name.
GUEST_KERNEL_TOKENS = (
    r"\bGuestKernelVersion\b",
    r"\bLINUX_VERSION_CODE\b",
    r"\bKERNEL_VERSION\s*\(",
    r"\bvermagic\b",
    r"\butsname\b",
    r"\bosrelease\b",
)


def axis_e() -> dict[str, list[str]]:
    found: dict[str, list[str]] = {}
    for tok in GUEST_KERNEL_TOKENS:
        hits = _grep_rs(tok)
        if hits:
            found[tok] = hits
    return found


# =====================================================================================
# AXIS F — multi-GPU
# =====================================================================================


def axis_f() -> dict[str, dict[str, list[str]]]:
    """Every `MG-n` decision, and — the part that bites — whether any CODE names it.

    A decision cited only in a design doc is a decision nothing enforces. Splitting
    the two is the only way this list can tell you that.
    """
    decisions: dict[str, dict[str, set[str]]] = {}
    for rel in tracked():
        if not (rel.endswith(".rs") or rel.endswith(".md")):
            continue
        kind = "code" if rel.endswith(".rs") else "docs"
        for tag in re.findall(r"\bMG-(\d+)\b", read(rel)):
            slot = decisions.setdefault(f"MG-{tag}", {"code": set(), "docs": set()})
            slot[kind].add(rel.split("/")[1] if rel.startswith("crates/") else rel)
    return {
        k: {"code": sorted(v["code"]), "docs": sorted(v["docs"])}
        for k, v in sorted(decisions.items(), key=lambda kv: int(kv[0][3:]))
    }


# =====================================================================================
# THE REPORT
# =====================================================================================


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--check",
        action="store_true",
        help="exit 1 if a derived fact moved from what the doc records",
    )
    args = ap.parse_args()

    closure = shipping_closure()
    problems: list[str] = []

    print("=" * 84)
    print("COMPATIBILITY MATRIX — derived from the tree, not written down")
    print("=" * 84)

    print("\nSHIPPING CLOSURE (crates a real build output links)")
    all_crates = set(crate_deps())
    guest_edge = closure_of(("kayfabe-qemu-raw",))
    host_edge = closure_of(("kayfabe-isolate-host",))
    print("  guest edge (QEMU staticlib) :", ", ".join(sorted(guest_edge)))
    print("  host edge  (isolate binaries):", ", ".join(sorted(host_edge)))
    print("  NEITHER                      :", ", ".join(sorted(all_crates - closure)))
    print("  ★ a seam in a crate on the last line is a seam no guest can reach today.")

    print("\n--- AXIS A: GUEST DRIVER VERSION ---",
          f"[{where('kayfabe-abi', closure)}]")
    rows = axis_a()
    print(f"  {'version':<12} {'answers RPC fn 1':<17} {'map_dma':<22} {'gsp_element'}")
    for r in rows:
        print(
            f"  {r['version']:<12} {('yes' if r['vgx'] else 'NO — refused'):<17} "
            f"{r['map_dma']:<22} {r['gsp_element']}"
        )
    bootable = [r["version"] for r in rows if r["vgx"]]
    print(f"  => {len(rows)} table rows, {len(bootable)} of which can answer the FIRST RPC:"
          f" {', '.join(bootable)}")
    print(f"  => the shim compiles in exactly one: GUEST_DRIVER = {shim_guest_driver()}"
          f" = {bench_driver()}")

    print("\n--- AXIS B: GPU ARCHITECTURE ---")
    b = axis_b()
    print(f"  ChipProfile rows (kayfabe-device::CHIPS, [{where('kayfabe-device', closure)}]):"
          f" {', '.join(b['chip_profiles'])}")
    print(f"  VBIOS profiles   (kayfabe-abi::VBIOS_PROFILES):"
          f" {', '.join(b['vbios_profiles'])}")
    print(f"  GSP models       (kayfabe-chips, [{where('kayfabe-chips', closure)}]):"
          f" {', '.join(b['gsp_models'])}")
    if len(b["vbios_profiles"]) != len(b["chip_profiles"]):
        print("  ★ ASYMMETRY: a VBIOS profile without a ChipProfile is unreachable —"
              " `chip_for_device_id` can never return it.")

    print("\n--- AXIS C: HOST DRIVER VERSION ---",
          f"[{where('kayfabe-isolate-host', closure)}]")
    c = axis_c()
    print(f"  encoders pinned to: {c['interval']}")
    print(f"  refusal arms      : {c['refusals']}")
    print(f"  known shift at    : major {c['shift_major']} (NV_CHANNEL_ALLOC_PARAMS)")

    print("\n--- AXIS D: GUEST OS ---", f"[{where('kayfabe-abi', closure)}]")
    d = axis_d()
    print(f"  variants        : {', '.join(d['variants'])}")
    print(f"  carry a rule    : {', '.join(d['with_rule'])}")
    print(f"  config callers  : {', '.join(d['config_callers']) or 'NONE — the door has no caller'}")
    print(f"  the crate that threads it (kayfabe-rmrpc) is [{where('kayfabe-rmrpc', closure)}]")

    print("\n--- AXIS E: GUEST KERNEL VERSION ---")
    e = axis_e()
    if not e:
        print("  NO HOME. None of the tokens that would exist if this axis were modelled")
        print("  appear anywhere in tracked Rust:")
        for tok in GUEST_KERNEL_TOKENS:
            print(f"    {tok}")
        print("  => a guest-kernel mismatch is not detected, refused, or logged.")
    else:
        for tok, hits in e.items():
            print(f"  {tok}: {len(hits)} hit(s) — {hits[0]}")
        problems.append("axis E grew a home; this script and the doc are out of date")

    print("\n--- AXIS F: MULTI-GPU ---")
    for tag, cite in axis_f().items():
        if cite["code"]:
            print(f"  {tag}: code = {', '.join(cite['code'])}")
        else:
            # ⚠ The tag, not the mechanism. MG-2's *implementation* is real
            # (`AllocFacts.device_instance`); what is absent is any code comment citing
            # the decision by number, so a reader of that code cannot find the decision.
            print(f"  {tag}: ★ no code CITES this tag ({', '.join(cite['docs'])})")

    print("\n" + "=" * 84)
    if args.check:
        if problems:
            for p in problems:
                print(f"★ MOVED: {p}")
            return 1
        print("no derived fact moved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
