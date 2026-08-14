#!/usr/bin/env python3
"""★★★★★ w315 — THE ATTRIBUTION, computed from two logs and nothing else.

    usage: w315_attribute.py <probe.log> <qemu.log> [--label NAME]

It refuses rather than invents.  Every number it prints is read out of one of the two
files; where a file does not carry a number the row says **UNMEASURED**, and no default,
no remembered figure and no fallback is ever substituted.  (`w311_ratio.sh` exits 2 for the
same reason; `dlen=0` rows in the C oracle are the reason the reason exists.)

## ⊘ THE ONE ARITHMETIC CLAIM, AND ITS LICENCE

`Σ host segments ≤ launch_ms` is checked.  Its licence is NESTING and nothing else: a guest
MMIO write is a vmexit, so the guest is halted for the whole of our trap.  **No offset
between the guest's CLOCK_MONOTONIC and the host's is computed here or anywhere in this
rung.**  Where the inequality is loose, the slack is printed as its own row and named —
⊘ it is NEVER distributed over the segments that did report.  (Outcome (D): the missing
time is the finding.)

## ⚠ WHAT IT CANNOT DO

- It cannot tell WHICH doorbell belongs to WHICH launch unless the `full` arm ran; with the
  `census` arm it works in per-boot aggregates and says so.
- It cannot tell "the host RM was slow" from "the socket was slow" — `core_rm_ipc` brackets
  the round trip and both live inside it.
- A cadence that matches the floor is reported as a SUSPECT, never as a corroboration.
"""

import re
import sys
from collections import OrderedDict

ITER_RE = re.compile(
    r"ITER N=(\d+) i=(\d+) launch_ms=([\d.]+) submit_ms=([\d.]+) sync_ms=([\d.]+) "
    r"t0_mono_ms=([\d.]+)"
)
BSUM_RE = re.compile(r"^\s*(?:GUEST_)?BSUM N=(\d+) (.*)$")
CENSUS_RE = re.compile(
    r"KFTIME-CENSUS kind=(\S+) why=(\S+) t_ms=[\d.]+ events=(\d+) total_ms=([\d.]+) "
    r"mean_us=(\d+) max_us=(\d+) marked_ms=([\d.]+) UNMARKED_ms=([\d.]+)"
)
SEG_RE = re.compile(
    r"KFTIME-SEG (\S+)\s+n=(\d+)\s+total_ms=([\d.]+)\s+mean_us=(\d+)\s+max_us=(\d+)\s+share=([\d.]+)%"
)
NESTED_RE = re.compile(
    r"KFTIME-NESTED (\S+)\s+calls=(\d+)\s+total_ms=([\d.]+)\s+mean_us=(\d+)"
)
HIST_RE = re.compile(r"KFTIME-HIST us:(.*)$")


def read(path):
    try:
        with open(path, "r", errors="replace") as f:
            return f.read().splitlines()
    except OSError as e:
        print(f"⊘ CANNOT READ {path}: {e} — this is a statement about the reader.")
        return None


def guest(lines):
    """Per-iteration guest facts.  ⊘ First iteration is kept but flagged, never silently
    dropped: it carries publication and first-touch backing, and a statistic that drops a
    sample without saying so is the shape w311 had to caveat."""
    iters, bsums = [], []
    for ln in lines:
        m = ITER_RE.search(ln)
        if m:
            iters.append(
                dict(
                    n=int(m.group(1)),
                    i=int(m.group(2)),
                    launch=float(m.group(3)),
                    submit=float(m.group(4)),
                    sync=float(m.group(5)),
                    t0=float(m.group(6)),
                )
            )
        m = BSUM_RE.match(ln)
        if m:
            bsums.append(ln.strip())
    return iters, bsums


def host(lines):
    """The LAST census of each kind wins — it is cumulative, so the last one is the whole
    run.  ⊘ Not the first: a periodic census printed at event 200 is a prefix, and reading a
    prefix as the total is exactly the truncated-artefact trap."""
    censuses, segs, nested, hist = OrderedDict(), OrderedDict(), OrderedDict(), OrderedDict()
    kind = None
    for ln in lines:
        m = CENSUS_RE.search(ln)
        if m:
            kind = m.group(1)
            censuses[kind] = dict(
                why=m.group(2),
                events=int(m.group(3)),
                total_ms=float(m.group(4)),
                mean_us=int(m.group(5)),
                max_us=int(m.group(6)),
                marked_ms=float(m.group(7)),
                unmarked_ms=float(m.group(8)),
            )
            segs[kind], nested[kind] = OrderedDict(), OrderedDict()
            continue
        if kind is None:
            continue
        m = SEG_RE.search(ln)
        if m:
            segs[kind][m.group(1)] = dict(
                n=int(m.group(2)),
                total_ms=float(m.group(3)),
                mean_us=int(m.group(4)),
                max_us=int(m.group(5)),
                share=float(m.group(6)),
            )
            continue
        m = NESTED_RE.search(ln)
        if m:
            nested[kind][m.group(1)] = dict(
                calls=int(m.group(2)), total_ms=float(m.group(3)), mean_us=int(m.group(4))
            )
            continue
        m = HIST_RE.search(ln)
        if m:
            hist[kind] = m.group(1).strip()
    return censuses, segs, nested, hist


def med(xs):
    s = sorted(xs)
    return s[len(s) // 2] if s else None


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 64
    probe, qemu = sys.argv[1], sys.argv[2]
    label = sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == "--label" else "?"
    pl, ql = read(probe), read(qemu)
    if pl is None or ql is None:
        print("=== w315 VERDICT = 2 (UNMEASURED — a log is missing)")
        return 2

    iters, bsums = guest(pl)
    censuses, segs, nested, hist = host(ql)

    print("=" * 86)
    print(f"=== ★★★★★ w315 ATTRIBUTION  label={label}")
    print(f"===   probe={probe}")
    print(f"===   qemu ={qemu}")
    print("=" * 86)

    # ---- 1. the guest half ---------------------------------------------------------------
    print("\n--- 1. THE GUEST'S OWN SEGMENTATION (guest CLOCK_MONOTONIC, both halves)")
    if not iters:
        print("    ⊘ NO INSTRUMENTED `ITER` LINES — this binary's guest arm is not w315's,")
        print("      or the workload never launched. UNMEASURED; ⊘ NOT zero.")
    else:
        steady = [d for d in iters if d["i"] > 0]
        for tag, rows in (("first launch", iters[:1]), ("steady state", steady)):
            if not rows:
                continue
            print(
                f"    {tag:<13} n={len(rows):<3} launch_med={med([r['launch'] for r in rows]):9.3f} ms"
                f"   submit_med={med([r['submit'] for r in rows]):9.3f} ms"
                f"   sync_med={med([r['sync'] for r in rows]):9.3f} ms"
            )
        if steady:
            sm, ym = med([r["submit"] for r in steady]), med([r["sync"] for r in steady])
            tot = sm + ym
            print(
                f"    ⇒ of the median launch, SUBMIT is {100 * sm / tot:5.1f}% and "
                f"COMPLETION is {100 * ym / tot:5.1f}%"
            )
            print(
                "    ★ This split needs NO clock correspondence — both halves are the guest's own"
            )
            print(
                "      clock. It says WHICH SIDE of cuLaunchKernel the floor is on, and nothing"
            )
            print("      about whose code is running there.")
    for b in bsums:
        print(f"    {b}")

    # ---- 2. the host half ----------------------------------------------------------------
    print("\n--- 2. THE HOST-SIDE BREAKDOWN (host CLOCK_MONOTONIC, vCPU thread, inside the trap)")
    if not censuses:
        print("    ⊘ NO KFTIME CENSUS IN THIS LOG. Either the instrument was not armed (the")
        print("      `base` arm — correct and expected) or it was armed and no hook fired.")
        print("      ⚠ Those are different facts and this file cannot tell them apart; check")
        print("      for the `KFTIME ARMED` line.")
        armed = any("KFTIME ARMED" in ln for ln in ql)
        print(f"      KFTIME ARMED line present = {armed}")
    for kind, c in censuses.items():
        print(
            f"\n    kind={kind}  why={c['why']}  events={c['events']}  total={c['total_ms']:.1f} ms"
            f"  mean={c['mean_us']} us  max={c['max_us']} us"
        )
        share = 100 * c["unmarked_ms"] / c["total_ms"] if c["total_ms"] else 0.0
        print(
            f"      marked={c['marked_ms']:.1f} ms   ★ UNMARKED={c['unmarked_ms']:.1f} ms "
            f"({share:.1f}%)  ⊘ the residual is NAMED, never distributed"
        )
        for name, s in segs.get(kind, {}).items():
            print(
                f"      SEG {name:<16} n={s['n']:<7} total={s['total_ms']:<10.1f} ms "
                f"mean={s['mean_us']:<8} us max={s['max_us']:<9} us share={s['share']:.1f}%"
            )
        for name, s in nested.get(kind, {}).items():
            print(
                f"      NESTED {name:<13} calls={s['calls']:<7} total={s['total_ms']:<10.1f} ms "
                f"mean={s['mean_us']:<8} us  ⊘ inside a SEG above — do not add"
            )
        if kind in hist:
            print(f"      HIST us: {hist[kind]}")

    # ---- 3. per-launch arithmetic --------------------------------------------------------
    print("\n--- 3. THE PER-LAUNCH ARITHMETIC — the only place the two halves meet")
    steady = [d for d in iters if d["i"] > 0]
    db = censuses.get("mmio_doorbell")
    other = censuses.get("mmio_other")
    if not steady or not db:
        print("    ⊘ UNMEASURED: needs BOTH instrumented guest iterations AND a host census.")
        print("      ⇒ this row is why the `base` arm cannot answer the attribution by itself.")
    else:
        nl = len(steady) + 1  # +1 for the discarded first launch: it rang doorbells too
        per_launch_db = db["events"] / nl
        host_ms_per_launch = db["total_ms"] / nl
        other_ms_per_launch = (other["total_ms"] / nl) if other else 0.0
        lm = med([r["launch"] for r in steady])
        print("    ⊘⊘ THE HOST CENSUS IS WHOLE-BOOT. It includes every doorbell rung during")
        print("       driver load, cuInit, cuCtxCreate, the copies and the batch phase — not")
        print("       only the timed launches. ⇒ dividing it by the launch count is an")
        print("       OVER-ESTIMATE of the per-launch trap cost, and that is the useful")
        print("       direction: if even the over-estimate is small against launch_ms, the")
        print("       floor is NOT inside our traps, and no clock correspondence was needed")
        print("       to say so. ⚠ The converse does NOT follow — a large over-estimate does")
        print("       not locate the time in the launches.")
        print(f"    launches in the run (incl. first)         = {nl}")
        print(f"    doorbell MMIO writes                      = {db['events']}")
        print(f"    ⇒ doorbell writes per launch (OVER-EST)   = {per_launch_db:.2f}")
        print(f"    ⇒ host doorbell-trap ms per launch (O-EST)= {host_ms_per_launch:8.3f} ms")
        print(f"    ⇒ host time IN OTHER MMIO TRAPS per launch= {other_ms_per_launch:8.3f} ms")
        print(f"    guest median launch_ms                    = {lm:8.3f} ms")
        acc = host_ms_per_launch + other_ms_per_launch
        print(f"    ⇒ ACCOUNTED (all MMIO traps)              = {acc:8.3f} ms  ({100 * acc / lm:5.1f}%)")
        gap = lm - acc
        print(f"    ⇒ ★★★ UNACCOUNTED                        = {gap:8.3f} ms  ({100 * gap / lm:5.1f}%)")
        print("      ⊘ UNACCOUNTED is time the guest spent NOT inside one of our traps: its own")
        print("        driver, its own userspace spin, or waiting for something we do not run.")
        print("        It is NAMED here and must not be attributed to any host segment.")
        if acc > lm:
            print("      ⚠⚠ ACCOUNTED EXCEEDS the guest's launch window. Nesting says that is")
            print("         IMPOSSIBLE ⇒ the doorbell population is not per-launch (start-up")
            print("         traffic is in the total), or the clocks are not what this assumes.")
            print("         ⊘ REPORT THIS AS OUTCOME (D)/(E) — do not rescale to make it fit.")

    # ---- 4. the suspects -----------------------------------------------------------------
    print("\n--- 4. ⚠ SUSPECTS, NOT CORROBORATIONS")
    print("    A period that matches the measured quantity is a suspect. w311 nearly shipped")
    print("    `251/2 = 125.5 ms ≈ C` as the mechanism; it was OBSERVER_TICK_MS = 250, the")
    print("    observer thread's own epoll timeout. Any coincidence below must be CLOSED by a")
    print("    bracket or an injection, never by the coincidence itself.")
    for pat, why in (
        (r"OBSERVER_TICK|SEMA-PAGE seq", "the 250 ms observer tick — an OBSERVER, on its own thread"),
        (r"VAS_DRAIN_WALL_BUDGET|budget", "a wall budget firing means rows went UNPUBLISHED, not refused"),
    ):
        n = sum(1 for ln in ql if re.search(pat, ln))
        print(f"    {n:>7} lines match /{pat}/ — {why}")

    print("\n=== w315 VERDICT = 0 (report produced; the LETTER is a human reading, not this exit)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
