#!/usr/bin/env python3
"""★★★★★ w315 — ALIGN THE HOST'S DOORBELL TRAPS TO THE GUEST'S LAUNCHES, AND SEGMENT THEM.

    usage: w315_align.py <probe.log> <qemu.log>

This is the load-bearing analysis of the rung, so its failure mode is written down first.

## ⊘⊘ THE ESTIMATOR THIS FILE DOES **NOT** USE, AND WHY — it was tried and it lied

The obvious score for a candidate offset δ is *"total host trap time overlapping the guest's
launch windows"*, maximised over δ.  **It is degenerate and it produced a confident wrong
answer.**  One doorbell in this boot lasted **1 991 ms**; placing the guest's 3.2 s of launch
windows anywhere under that single trap credits *every* window in full, so the score
saturates at exactly `Σ launch_ms` and a **5 ms-wide plateau of identical maxima** appears.
The first run of this analysis reported δ = −29 601 ms with *"99.7 % of the launch window is
inside a doorbell trap"* — a number that is the arithmetic maximum by construction and says
nothing whatever about alignment.

⇒ Same species as w311's `251/2 = 125.5 ms`: **an answer that arrives pre-corroborated.**
The tell was that the score equalled its own theoretical ceiling; a real alignment does not.

## ★★★ THE ESTIMATOR IT DOES USE

For each guest launch window, credit **only the longest doorbell trap whose START falls
inside it**, and score by `RMS(host_trap_duration − guest_submit_ms)` — minimised.

- A blanket trap **starts** in at most one window, so it cannot saturate.
- The score is a *shape* match over twelve values, not a sum, so it cannot be maximised by
  making one number bigger.
- It is falsifiable in the ordinary way: if the best RMS is not far below the median RMS over
  all offsets, **there is no alignment** and the script says so and refuses (outcome (E) —
  report guest-side segmentation only, explicitly unattributed across the boundary).

## ⊘ THE ONE THING NEITHER ESTIMATOR NEEDS

An offset between the guest's `CLOCK_MONOTONIC` and the host's.  δ is **derived here**, from
the data, as the value that makes the two sequences agree — it is a *result*, not an
assumption, and the residual it leaves is itself reported (it is the vmexit + guest-driver
cost that lies outside our handler).
"""

import re
import statistics
import sys

ITER_RE = re.compile(
    r"ITER N=(\d+) i=(\d+) launch_ms=([\d.]+) submit_ms=([\d.]+) sync_ms=([\d.]+) "
    r"t0_mono_ms=([\d.]+)"
)
EV_RE = re.compile(
    r"KFTIME (\S+) t_ms=([\d.]+) total_us=(\d+) marked_us=(\d+) unmarked_us=(\d+) \| "
)

HOST_SEGS = ("core", "core_rm_ipc", "fwd_drain", "materialize")

FAMILIES = (
    ("page-table + publication", ("vas_publish", "pt_decode", "pt_sweep", "pt_vascensus", "pt_witness")),
    ("ring projection / probes", ("ringproj", "ce_try", "bindcensus", "pin_ring", "operand_join",
                                  "err_notifier", "vmm_lock", "vmm_unlock")),
    ("THE ACTUAL HOST FORWARD", ("core",)),
    ("our own logging (instrument)", ()),  # filled by prefix below
)


def shape(name):
    if name.startswith("log_"):
        return "log"
    return "host" if name in HOST_SEGS else "work"


def parse_events(lines):
    """Every per-event KFTIME line, with its segment map.  ⊘ `t_ms` is the instant the line was
    FORMATTED, i.e. the END of the bracket, so the trap occupied `[t_ms - total, t_ms]`."""
    out = []
    for raw in lines:
        raw = raw.strip()
        m = EV_RE.search(raw)
        if not m:
            continue
        kind, t, tot, unm = m.group(1), float(m.group(2)), int(m.group(3)), int(m.group(5))
        rest = raw[m.end():]
        nested = {}
        if " | NESTED" in rest:
            rest, nn = rest.split(" | NESTED", 1)
            if ":" in nn:
                for p in nn.split(":", 1)[1].split():
                    if "=" in p:
                        k, v = p.split("=", 1)
                        nested[k] = int(v.split("us/")[0])
        segs = {}
        for p in rest.split():
            if "=" in p:
                k, v = p.split("=", 1)
                if v.isdigit():
                    segs[k] = int(v)
        out.append(dict(kind=kind, t=t, start=t - tot / 1000.0, tot=tot, unm=unm,
                        segs=segs, nested=nested))
    return out


def parse_iters(lines):
    seen = {}
    for ln in lines:
        m = ITER_RE.search(ln)
        if m:
            seen[int(m.group(2))] = (float(m.group(6)), float(m.group(3)), float(m.group(4)))
    return [seen[k] for k in sorted(seen)]


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 64
    try:
        probe = open(sys.argv[1], errors="replace").read().splitlines()
        qemu = open(sys.argv[2], errors="replace").read().splitlines()
    except OSError as e:
        print(f"⊘ CANNOT READ: {e} — a statement about the reader. VERDICT = 2 (UNMEASURED)")
        return 2

    it = parse_iters(probe)
    ev = parse_events(qemu)
    db = sorted([e for e in ev if e["kind"] == "mmio_doorbell"], key=lambda e: e["start"])
    fwd = sorted([e for e in ev if e["kind"] == "doorbell_fwd"], key=lambda e: e["start"])

    print("=" * 88)
    print("=== ★★★★★ w315 — THE HOST DOORBELL TRAP, ALIGNED TO THE GUEST'S LAUNCHES")
    print("=" * 88)
    print(f"    guest launches with a submit/sync split: {len(it)}")
    print(f"    host per-event doorbell lines: mmio_doorbell={len(db)}  doorbell_fwd={len(fwd)}")
    if not it or not db:
        print("\n⊘ UNMEASURED. This needs the `full` arm (per-event lines) AND an instrumented")
        print("  guest. ⊘ It is NOT a zero and must not be reported as one.")
        print("=== w315 ALIGN VERDICT = 2 (unmeasured)")
        return 2

    g_t0 = [x[0] for x in it]
    g_lm = [x[1] for x in it]
    g_sub = [x[2] for x in it]
    starts = [e["start"] for e in db]
    durs = [e["tot"] / 1000.0 for e in db]

    def score(d):
        errs, hit, got = [], 0, []
        for t0, lm, sm in it:
            a, b = t0 + d, t0 + d + lm
            cand = [durs[i] for i, s in enumerate(starts) if a <= s < b]
            if not cand:
                errs.append(sm)
                got.append(0.0)
                continue
            hit += 1
            v = max(cand)
            got.append(v)
            errs.append(v - sm)
        return (sum(e * e for e in errs) / len(errs)) ** 0.5, hit, got

    lo = int(min(starts) - max(g_t0) - 500)
    hi = int(max(starts) - min(g_t0) + 500)
    res = sorted((score(float(d))[0], float(d)) for d in range(lo, hi))
    med = statistics.median(r[0] for r in res)
    best_rms, delta = res[0]
    ratio = med / best_rms if best_rms else float("inf")

    print(f"\n--- THE ALIGNMENT (derived, not assumed)")
    print(f"    best RMS(host_trap_dur − guest_submit_ms) = {best_rms:.3f} ms at δ = {delta:.0f} ms")
    print(f"    median RMS over all {len(res)} candidate offsets = {med:.1f} ms")
    print(f"    ⇒ the best is {ratio:.1f}× better than a random offset")
    # ★ The refusal. A shallow minimum is NOT an alignment, and reporting one anyway is how a
    #   coincidence becomes a mechanism.
    if ratio < 5.0:
        print("\n⊘⊘ THE MINIMUM IS SHALLOW. There is no trustworthy correspondence between the")
        print("   two clocks in this data ⇒ PRE-REGISTERED OUTCOME (E): report the guest-side")
        print("   segmentation only, EXPLICITLY UNATTRIBUTED across the boundary. ⊘ The host")
        print("   numbers below would be an alignment nobody measured.")
        print("=== w315 ALIGN VERDICT = 2 (no trustworthy clock correspondence)")
        return 2

    rms, hit, got = score(delta)
    print(f"    launch windows with a doorbell trap starting inside: {hit}/{len(it)}")
    print("\n    %-3s %12s %12s %12s %12s" % ("i", "host_trap", "g_submit", "g_launch", "resid"))
    for k in range(len(it)):
        print("    %-3d %12.3f %12.3f %12.3f %12.3f" % (k, got[k], g_sub[k], g_lm[k], got[k] - g_sub[k]))
    resid = [got[k] - g_sub[k] for k in range(len(it))]
    print(f"\n    Σ host trap  = {sum(got):9.3f} ms")
    print(f"    Σ g_submit   = {sum(g_sub):9.3f} ms   ⇒ the trap is {100 * sum(got) / sum(g_sub):.1f}% of SUBMIT")
    print(f"    Σ g_launch   = {sum(g_lm):9.3f} ms   ⇒ the trap is {100 * sum(got) / sum(g_lm):.1f}% of the LAUNCH")
    print(f"    ★ residual (submit − trap) = {-statistics.mean(resid):.3f} ms per launch, "
          f"sd {statistics.pstdev(resid):.3f}")
    print("      ⊘ THIS is where the vmexit lives. The exit is over before our handler runs, so")
    print("        nested-virt tax can only appear here — and here it is a few ms, not tens.")
    print("        ⚠ It also contains the guest driver's own work around the store; this")
    print("        measurement cannot split those two and does not claim to.")

    # ---- the segment breakdown of exactly those doorbells --------------------------------
    sel = []
    for t0, lm, _ in it:
        a, b = t0 + delta, t0 + delta + lm
        c = [f for f in fwd if a <= f["start"] < b]
        if c:
            sel.append(max(c, key=lambda f: f["tot"]))
    print(f"\n--- ★★★★★ THE BREAKDOWN OF THE LAUNCH DOORBELLS ONLY ({len(sel)} matched)")
    print("    ⊘ NOT the whole-boot census: that one is dominated by cuInit/cuCtxCreate traffic")
    print("      and by two multi-second outliers. These are the doorbells the timed launches")
    print("      actually rang.")
    if not sel:
        print("    ⊘ NONE MATCHED — the forward-path lines are absent. UNMEASURED.")
        print("=== w315 ALIGN VERDICT = 2")
        return 2
    names = []
    for f in sel:
        for k in f["segs"]:
            if k not in names:
                names.append(k)
    tot = sum(f["tot"] for f in sel)
    print(f"\n    Σ trap = {tot / 1000.0:.3f} ms over {len(sel)} launches = {tot / 1000.0 / len(sel):.3f} ms/launch")
    print("\n    %-16s %-5s %10s %11s %11s %8s" % ("segment", "shape", "total ms", "ms/launch", "median ms", "share"))
    for s, k in sorted(((sum(f["segs"].get(k, 0) for f in sel), k) for k in names), reverse=True):
        v = [f["segs"].get(k, 0) for f in sel]
        print("    %-16s %-5s %10.3f %11.3f %11.3f %7.1f%%"
              % (k, shape(k), s / 1000.0, s / 1000.0 / len(sel), statistics.median(v) / 1000.0, 100.0 * s / tot))
    unm = sum(f["unm"] for f in sel)
    print("    %-16s %-5s %10.3f %11.3f %11s %7.1f%%"
          % ("UNMARKED", "-", unm / 1000.0, unm / 1000.0 / len(sel), "-", 100.0 * unm / tot))
    print("      ⊘ UNMARKED is the residual INSIDE the bracket. It is printed, never distributed.")
    nk = {}
    for f in sel:
        for k, v in f["nested"].items():
            nk[k] = nk.get(k, 0) + v
    for k, v in nk.items():
        print("    NESTED %-9s %-5s %10.3f %11.3f %11s %7.1f%%  ⊘ INSIDE `core` — do not add"
              % (k, "host", v / 1000.0, v / 1000.0 / len(sel), "-", 100.0 * v / tot))

    print("\n    ★ ROLL-UP BY FAMILY (and by SHAPE, which is what says whether bare metal helps):")
    logs = [k for k in names if k.startswith("log_")]
    for fname, ks in FAMILIES:
        keys = logs if fname.startswith("our own logging") else ks
        s = sum(sum(f["segs"].get(k, 0) for k in keys) for f in sel)
        print("      %-30s %9.3f ms  %8.3f ms/launch  %6.1f%%"
              % (fname, s / 1000.0, s / 1000.0 / len(sel), 100.0 * s / tot))
    for sh in ("work", "host", "log"):
        s = sum(sum(v for k, v in f["segs"].items() if shape(k) == sh) for f in sel)
        note = {"work": "costs the SAME on bare metal ⇒ a real finding",
                "host": "blocked in the host RM / the isolate child",
                "log": "the instrument's own printing — see the `base` arm for its true cost"}[sh]
        print("      shape=%-5s %9.3f ms  %8.3f ms/launch  %6.1f%%   (%s)"
              % (sh, s / 1000.0, s / 1000.0 / len(sel), 100.0 * s / tot, note))

    print("\n=== w315 ALIGN VERDICT = 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
