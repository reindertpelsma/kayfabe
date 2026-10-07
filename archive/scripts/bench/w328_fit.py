#!/usr/bin/env python3
"""★★★★★ w328 — attribute the worst trap, and price the breadth, from a boot's own log.

⚠ THE TRAP THIS SCRIPT IS WRITTEN AGAINST, and it is this campaign's own, banked five times
in one day: *a candidate whose magnitude matches your measurement belongs to the INSTRUMENT
until proven otherwise.* ⇒ every residual is printed, never summarised; the fit is refused
below 3 unclamped points; and a budget-CLAMPED point carries no independent information about
the fit and is excluded from it while still being printed.

⊘ AND THE SECOND ONE: an absent number is UNMEASURED, never 0. Every field below prints
`⊘UNMEASURED` when its line is missing, and a boot missing a field is excluded from the fit
rather than contributing a zero.

usage:  w328_fit.py <qemu.log> [<qemu.log> ...]
        w328_fit.py --selftest
"""

import re
import sys

# The wall budget the drain is measured against, from the source
# (`VAS_DRAIN_WALL_BUDGET`, crates/kayfabe-qemu-raw/src/shim.rs). ⊘ NOT fitted — a constant
# read out of the tree, so "margin" means the same thing here as it does in the code.
BUDGET_MS = 3000

RE_WORST = re.compile(r"worst_trap=(\d+)us")
RE_DRAIN = re.compile(
    r"DRAIN\[visited=true asked=(\d+) pinned=(\d+) refused=(\d+) DRAIN_MS=(\d+) "
    r"W319KNOB\[[^\]]*\] complete=(true|false)"
)
RE_SCOPE = re.compile(
    r"W328SCOPE\[arm=(\S+) scoped=(\S+) target=(.*?) scoped_out=(\d+) target_us=(\d+) "
    r"target_published=(\d+) other_vases=(\d+) other_us=(\d+) other_published=(\d+) "
    r"other_refused=(\d+) breadth_share=(\S+?)\]"
)
RE_PIN = re.compile(
    r"W328PIN\[arm=(\S+) scoped=(\S+) scoped_out=(\d+) other_vases=(\d+) "
    r"other_us=(\d+) other_pinned=(\d+) drain_ms=(\d+)\]"
)


def parse(path):
    """One boot → one dict. ⊘ Missing keys are ABSENT, never defaulted to 0."""
    try:
        with open(path, "rb") as fh:
            text = fh.read().decode("utf-8", "replace")
    except OSError as e:
        return {"path": path, "error": str(e)}

    out = {"path": path}

    # ★ worst_trap is WHOLE-PROCESS and MONOTONIC (trapwitness.rs), so the LAST census line
    #   carries the boot's maximum. Taking the first would report an early trap as the worst.
    w = RE_WORST.findall(text)
    if w:
        out["worst_trap_us"] = int(w[-1])

    # ★ The FIRST drain clause with a non-zero DRAIN_MS. `guest_ram_publication_merge.md`
    #   measured the shape: one real drain, a handful of small follow-ups, then ~180
    #   doorbells at DRAIN_MS=0. `tail -1` would report the boot's drain as free.
    for m in RE_DRAIN.finditer(text):
        asked, pinned, refused, ms, complete = m.groups()
        if int(ms) > 0 or int(asked) > 0:
            out.setdefault(
                "drain",
                {
                    "asked": int(asked),
                    "pinned": int(pinned),
                    "refused": int(refused),
                    "ms": int(ms),
                    "complete": complete == "true",
                },
            )

    # ★★ THE BREADTH, TWO WAYS, because they answer different questions:
    #   `first`      — the breadth's share of the WORST trap (the one-shot drain doorbell).
    #   `cumulative` — the breadth's share of the whole boot's publication BQL, which is what
    #                  the "229 passes / 2 529 ms" figure is about. Reporting one as the other
    #                  is exactly the conflation this rung exists to undo.
    scopes = [m.groups() for m in RE_SCOPE.finditer(text)]
    if scopes:
        out["scope_passes"] = len(scopes)
        out["scope_arm"] = scopes[0][0]
        out["scope_first"] = {
            "scoped": scopes[0][1] == "true",
            "scoped_out": int(scopes[0][3]),
            "target_us": int(scopes[0][4]),
            "other_vases": int(scopes[0][6]),
            "other_us": int(scopes[0][7]),
            "other_published": int(scopes[0][8]),
        }
        out["scope_cum"] = {
            "target_us": sum(int(g[4]) for g in scopes),
            "other_us": sum(int(g[7]) for g in scopes),
            "other_published": sum(int(g[8]) for g in scopes),
            "target_published": sum(int(g[5]) for g in scopes),
            "scoped_out": sum(int(g[3]) for g in scopes),
        }

    pins = [m.groups() for m in RE_PIN.finditer(text)]
    if pins:
        out["pin_passes"] = len(pins)
        # The pin pass that actually drained — the one whose `drain_ms` is the worst trap's.
        drained = [g for g in pins if int(g[6]) > 0]
        src = drained[0] if drained else pins[0]
        out["pin_first"] = {
            "scoped": src[1] == "true",
            "scoped_out": int(src[2]),
            "other_vases": int(src[3]),
            "other_us": int(src[4]),
            "other_pinned": int(src[5]),
            "drain_ms": int(src[6]),
        }
        out["pin_cum"] = {
            "other_us": sum(int(g[4]) for g in pins),
            "other_pinned": sum(int(g[5]) for g in pins),
        }
    return out


def u(v, fmt="{}"):
    return "⊘UNMEASURED" if v is None else fmt.format(v)


def report(boots):
    print("=" * 96)
    print("=== ★★★★★ w328 — WHICH 'DRAIN' EVERY NUMBER IS, STATED PER COLUMN")
    print("===   drain_ms      = the DOORBELLED VAS's guest-RAM pin drain (shim.rs `if doorbelled`)")
    print("===   pin_other_us  = the SAMPLED (non-doorbelled) VASes' 256-row pins — THE BREADTH")
    print("===   pub_other_us  = the publication CENSUS pass over non-doorbelled VASes — THE BREADTH")
    print("===   residual      = worst_trap - drain_us, i.e. EVERYTHING ELSE in that one trap")
    print("=" * 96)
    hdr = (
        f"{'boot':<26}{'worst_us':>11}{'drain_ms':>10}{'resid_us':>10}"
        f"{'drain%':>8}{'margin':>8}{'compl':>7}{'pin/ask':>14}"
    )
    print(hdr)
    print("-" * len(hdr))
    fitpts = []
    for b in boots:
        name = b["path"].split("/")[-1].replace("run_", "").replace("_qemu.log", "")
        d = b.get("drain")
        w = b.get("worst_trap_us")
        if d is None or w is None:
            print(f"{name:<26}{u(w):>11}{'⊘':>10}{'⊘':>10}{'⊘':>8}{'⊘':>8}{'⊘':>7}{'⊘':>14}")
            continue
        drain_us = d["ms"] * 1000
        resid = w - drain_us
        share = 100.0 * drain_us / w if w else 0.0
        margin = BUDGET_MS / d["ms"] if d["ms"] else float("inf")
        clamped = d["ms"] >= BUDGET_MS
        pa = "{}/{}".format(d["pinned"], d["asked"])
        print(
            f"{name:<26}{w:>11}{d['ms']:>10}{resid:>10}{share:>7.1f}%"
            f"{margin:>7.2f}x{str(d['complete']):>7}{pa:>14}"
            + ("  ⚠ BUDGET-CLAMPED" if clamped else "")
        )
        if not clamped:
            fitpts.append((name, resid, share))

    print()
    print("=== ★★★ THE ATTRIBUTION, on UNCLAMPED points only — every residual printed")
    if len(fitpts) < 3:
        print(
            f"    ⊘ REFUSED: {len(fitpts)} unclamped point(s), need ≥3. A share computed from "
            "fewer is a number, not a fit."
        )
    else:
        for n, r, s in fitpts:
            print(f"    {n:<24} residual={r:>9} us   drain share={s:.1f}%")
        lo, hi = min(s for _, _, s in fitpts), max(s for _, _, s in fitpts)
        print(f"    ⇒ drain share {lo:.1f}–{hi:.1f}% over {len(fitpts)} unclamped points")
        drop = sorted(fitpts, key=lambda t: -t[1])[1:]
        lo2, hi2 = min(s for _, _, s in drop), max(s for _, _, s in drop)
        print(
            f"    ⇒ REFIT WITHOUT THE LARGEST RESIDUAL ({len(drop)} points): {lo2:.1f}–{hi2:.1f}%"
            "  ★ the fit survives the drop" if abs(lo2 - lo) < 5 else ""
        )

    print()
    print("=== ★★★★★ THE BREADTH — what sweeping every live pid × every VAS key COSTS and DELIVERS")
    hdr2 = (
        f"{'boot':<26}{'arm':>11}{'passes':>8}{'pub_oth_us':>12}{'pub_oth_pub':>12}"
        f"{'pin_oth_us':>12}{'pin_oth_pin':>12}{'sc_out':>8}"
    )
    print(hdr2)
    print("-" * len(hdr2))
    for b in boots:
        name = b["path"].split("/")[-1].replace("run_", "").replace("_qemu.log", "")
        sc, pc = b.get("scope_cum"), b.get("pin_cum")
        if sc is None and pc is None:
            print(
                f"{name:<26}{'⊘UNMEASURED — no W328 line; OLD BINARY, ⊘ NOT zero breadth':>60}"
            )
            continue
        print(
            f"{name:<26}{b.get('scope_arm', '⊘'):>11}{b.get('scope_passes', 0):>8}"
            f"{sc['other_us'] if sc else 0:>12}{sc['other_published'] if sc else 0:>12}"
            f"{pc['other_us'] if pc else 0:>12}{pc['other_pinned'] if pc else 0:>12}"
            f"{(sc['scoped_out'] if sc else 0):>8}"
        )
    print()
    print("=== ★★★ AND THE ONE RATIO THE BRIEF TURNS ON, per boot:")
    print("===   breadth's share of the WORST TRAP  =  (pub_other_us + pin_other_us on that trap)")
    print("===                                        / worst_trap_us")
    for b in boots:
        name = b["path"].split("/")[-1].replace("run_", "").replace("_qemu.log", "")
        w = b.get("worst_trap_us")
        sf, pf = b.get("scope_first"), b.get("pin_first")
        if w is None or (sf is None and pf is None):
            print(f"    {name:<24} ⊘UNMEASURED")
            continue
        br = (sf["other_us"] if sf else 0) + (pf["other_us"] if pf else 0)
        print(
            f"    {name:<24} breadth={br:>9} us of worst_trap={w:>9} us"
            f"  ⇒ {100.0 * br / w:.2f}%"
        )


def selftest():
    """⊘ The matcher against the PRODUCTION SHAPE, not against a shape I invented.

    w319's attributor passed its own selftest 5/5 while broken on every real log, because its
    fixtures had no NESTED BRACKET and production lines do. These fixtures carry the nested
    `W319KNOB[...]` verbatim for exactly that reason.
    """
    fix = (
        "kayfabe: VAS-PUBLISH token=0x00020013 arm=drain "
        "W328SCOPE[arm=all scoped=false target=proc=2 pdb=0x6000 scoped_out=0 "
        "target_us=41000 target_published=3 other_vases=3 other_us=9000 "
        "other_published=0 other_refused=8 breadth_share=18%] gate=on → published=3 refused=8\n"
        "kayfabe: VAS-PUBLISH PINRATE → DRAIN[visited=true asked=13313 pinned=13313 "
        "refused=0 DRAIN_MS=2792 W319KNOB[budget_ms=3000 row_limit=65536] complete=true ] "
        "W328PIN[arm=all scoped=false scoped_out=0 other_vases=3 other_us=31000 "
        "other_pinned=768 drain_ms=2792]\n"
        "kayfabe: TRAPWITNESS off_trap_claims=0 inline_exceptions=0 worst_trap=2879349us\n"
    )
    import tempfile, os

    fd, p = tempfile.mkstemp(suffix="_qemu.log")
    os.write(fd, fix.encode())
    os.close(fd)
    b = parse(p)
    os.unlink(p)
    ok = True

    def chk(name, got, want):
        nonlocal ok
        if got != want:
            print(f"  ✘ {name}: got {got!r} want {want!r}")
            ok = False
        else:
            print(f"  ✔ {name} ⇒ {got}")

    chk("worst_trap_us", b.get("worst_trap_us"), 2879349)
    chk("drain.ms", b.get("drain", {}).get("ms"), 2792)
    chk("drain.complete", b.get("drain", {}).get("complete"), True)
    chk("drain.pinned==asked", b.get("drain", {}).get("pinned") == b.get("drain", {}).get("asked"), True)
    # ★★★ THE NESTED-BRACKET ASSERTION, the one w319 paid for: `complete=` sits AFTER
    #     `W319KNOB[...]`, so a `[^]]*` matcher never reaches it.
    chk("nested-bracket extraction reached complete=", "complete" in str(b.get("drain")), True)
    chk("scope_first.other_us", b.get("scope_first", {}).get("other_us"), 9000)
    chk("scope_first.target_us", b.get("scope_first", {}).get("target_us"), 41000)
    chk("pin_first.other_us", b.get("pin_first", {}).get("other_us"), 31000)
    chk("pin_first.drain_ms", b.get("pin_first", {}).get("drain_ms"), 2792)
    # ⊘ A missing field must be ABSENT, not 0.
    b2 = parse("/nonexistent/run_x_qemu.log")
    chk("missing log ⇒ no worst_trap key", "worst_trap_us" not in b2, True)
    print("SELFTEST " + ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--selftest":
        sys.exit(selftest())
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    report([parse(p) for p in sys.argv[1:]])
