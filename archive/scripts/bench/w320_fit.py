#!/usr/bin/env python3
"""★★★★★ w320 — SEPARATE THE PER-LAUNCH TERM FROM THE PER-SYNC TERM.

    usage: w320_fit.py <probe.log> [<probe.log> ...]
           w320_fit.py --selftest

Reads `BSWEEPSUM` / `BSWEEP` / `ITER` / `BSUM` rows (with or without a `GUEST_` prefix) and
fits `total(K) = a*K + b` by ordinary least squares over the per-K medians.

## ⊘⊘ WHY THE FIT IS PRINTED WITH ITS RESIDUALS AND NOT AS TWO NUMBERS

w311 fitted a two-parameter model to TWO points. Two points determine a line exactly, so the
fit had **no residual at all** and nothing in it could have disagreed with the data. This
script therefore

  - REFUSES to fit fewer than 3 distinct K (`⊘ UNDERDETERMINED`, not a number);
  - prints every residual, the RMS, and the WORST point by name;
  - prints the same fit with the largest K DROPPED. If `a` and `b` move a lot, the curve is
    not linear and the two-term reading is the wrong model — say so instead of quoting `b`.

★ A linear fit that is never allowed to fail is the same instrument class as w315's
overlap-maximising aligner (which saturated at its own arithmetic ceiling) and w311's 251 ms
cadence (which matched to 0.4 % because it WAS the observer). **The residual is the part that
can disagree**, so it is the part that gets printed.

## The reading, stated before any data is loaded

    slope a  ~ the SUBMIT floor (w318: 4.04 ms)   and  intercept b ~ 20-25 ms
        => the wait is PER-SYNC. Batching amortises it. w311's "batching does not help" is dead.
    slope a  ~ submit + sync (~27.5 ms)           and  intercept b ~ 0
        => the wait is PER-LAUNCH real work. Batching moves nothing and the floor is honest.
    anything else => report BOTH terms and the residual; do not round to whichever story is tidier.
"""
import re, sys

SWEEP_RE = re.compile(
    r"(?:GUEST_)?BSWEEPSUM N=(\d+) K=(\d+) reps=(\d+) med_total_ms=([\d.-]+) "
    r"per_launch_ms=([\d.-]+) min_ms=([\d.-]+) max_ms=([\d.-]+) med_cpu_ms=([\d.-]+) "
    r"med_offcpu_ms=([\d.-]+) med_nvcsw=([\d.-]+) bad=(-?\d+)")
ITER_RE = re.compile(
    r"ITER N=(\d+) i=(\d+) launch_ms=([\d.]+) submit_ms=([\d.]+) sync_ms=([\d.]+) "
    r"t0_mono_ms=([\d.]+) sub_cpu_ms=([\d.-]+) syn_cpu_ms=([\d.-]+) syn_offcpu_ms=([\d.-]+) "
    r"syn_nvcsw=(-?\d+) syn_nivcsw=(-?\d+)")
BSUM_RE = re.compile(r"(?:GUEST_)?BSUM N=(\d+) (.*)$")


def ols(xs, ys):
    """slope, intercept. Plain OLS; no library, so the arithmetic is auditable here."""
    n = len(xs)
    mx = sum(xs) / n
    my = sum(ys) / n
    den = sum((x - mx) ** 2 for x in xs)
    if den == 0:
        return None, None
    a = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den
    return a, my - a * mx


def fit_block(pts, label):
    """pts: list of (K, total_ms). Prints the fit or refuses."""
    ks = sorted({k for k, _ in pts})
    if len(ks) < 3:
        print(f"    ⊘ {label}: UNDERDETERMINED — {len(ks)} distinct K. A two-parameter model")
        print(f"      over <3 points fits EXACTLY and leaves no residual that could disagree.")
        print(f"      This is refused, not estimated. (w311 paid for this.)")
        return None
    xs = [k for k, _ in pts]
    ys = [t for _, t in pts]
    a, b = ols(xs, ys)
    if a is None:
        print(f"    ⊘ {label}: degenerate (all K equal)")
        return None
    res = [(k, t, t - (a * k + b)) for k, t in pts]
    rms = (sum(r * r for _, _, r in res) / len(res)) ** 0.5
    worst = max(res, key=lambda r: abs(r[2]))
    print(f"    {label}:  total(K) = {a:.4f}*K + {b:.3f} ms      "
          f"[n={len(pts)}, K={min(xs)}..{max(xs)}]")
    print(f"        slope  a = {a:8.4f} ms per LAUNCH")
    print(f"        interc b = {b:8.3f} ms per SYNC")
    print(f"        RMS residual = {rms:.3f} ms   worst: K={worst[0]} "
          f"obs={worst[1]:.3f} resid={worst[2]:+.3f}")
    for k, t, r in sorted(res):
        print(f"          K={k:<5d} obs={t:10.3f}  fit={a*k+b:10.3f}  resid={r:+8.3f}")
    return a, b, rms


def main(paths):
    # ⊘⊘ DEDUPE, AND IT IS NOT COSMETIC. The hook prints the workload's output TWICE: once
    # verbatim and INDENTED, once re-emitted at column 0 with a GUEST_ prefix. An unanchored
    # regex matches both, so the first version of this script reported `n=16` for an 8-point
    # sweep and drew a fit through every point twice. ⚠ OLS is invariant to exact duplication,
    # so the FIT looked fine and only `n` was wrong — which is worse than a visible error: the
    # residual count, and therefore any confidence anyone reads into it, was doubled for free.
    # ★ Same family as w307's inverted indent trap, arriving from the other side.
    seen_sw, seen_it = set(), set()
    sweeps, iters, bsums = [], [], []
    for p in paths:
        for ln in open(p, errors="replace"):
            m = SWEEP_RE.search(ln)
            if m:
                key = (p,) + m.groups()
                if key not in seen_sw:
                    seen_sw.add(key)
                    sweeps.append(tuple(m.groups()))
                continue
            m = ITER_RE.search(ln)
            if m:
                key = (p,) + m.groups()
                if key not in seen_it:
                    seen_it.add(key)
                    iters.append(tuple(m.groups()))
                continue
            m = BSUM_RE.search(ln)
            if m:
                bsums.append((int(m.group(1)), m.group(2)))

    print("=" * 78)
    print("★★★★★ w320 — THE PER-LAUNCH TERM vs THE PER-SYNC TERM")
    print("=" * 78)
    print(f"  inputs: {', '.join(paths)}")
    print(f"  BSWEEPSUM rows={len(sweeps)}  ITER rows={len(iters)}  BSUM rows={len(bsums)}")
    if not sweeps and not iters:
        print("  ⊘⊘ NOTHING PARSED. This is a statement about the LOG, not about the plane:")
        print("     either the workload is not w320's cup8bench, or the run produced no rows.")
        return 2

    # ---- the sweep fit, per size --------------------------------------------------------
    if sweeps:
        print("\n★★★★★ THE BATCH SWEEP — total(K) = a*K + b")
        bysize = {}
        for (N, K, reps, med, perl, mn, mx, cpu, off, csw, bad) in sweeps:
            bysize.setdefault(int(N), []).append(
                (int(K), float(med), float(cpu), float(off), float(csw), int(bad)))
        for N in sorted(bysize):
            rows = sorted(bysize[N])
            print(f"\n  --- N={N} ---")
            print(f"    {'K':>5} {'med_total':>11} {'per_launch':>11} {'cpu':>9} "
                  f"{'offcpu':>9} {'nvcsw':>7} {'bad':>6}")
            for (K, med, cpu, off, csw, bad) in rows:
                flag = "" if bad == 0 else "   ✘ BAD"
                print(f"    {K:>5} {med:>11.3f} {med/K:>11.3f} {cpu:>9.3f} "
                      f"{off:>9.3f} {csw:>7.1f} {bad:>6}{flag}")
            tb = sum(b for *_, b in rows)
            print(f"    Σbad over the sweep = {tb}  "
                  f"{'✔ every batched rep verified' if tb == 0 else '✘✘ A BATCHED REP FAILED'}")
            print()
            # ★★★★★ READ THIS COLUMN BEFORE THE FIT. `per_launch = total/K` is the NORMALISED
            # form, and it answers the rung's question without a model at all:
            #   FLAT across K            => the cost is PER-LAUNCH; the per-sync term is ~0;
            #                               batching CANNOT help, whatever a fit's intercept says.
            #   FALLING as 1/K           => a fixed PER-SYNC term dominates; batching amortises it.
            #   FALLING, but not as 1/K  => something else gets cheaper with K. NAME IT; do not
            #                               bank it as the intercept of a line it does not fit.
            # ⊘ A two-term fit will ALWAYS return an intercept, including for data that has no
            #   fixed term at all. The intercept is only meaningful if the residuals are small,
            #   which is why they are printed and why this column is printed first.
            pl = [(K, med / K) for (K, med, *_) in rows]
            lo = min(v for _, v in pl)
            hi = max(v for _, v in pl)
            print("    ★ PER-LAUNCH (total/K) — the model-free reading:")
            for K, v in pl:
                print(f"      K={K:<5d} {v:8.3f} ms/launch")
            print(f"      spread {lo:.3f}..{hi:.3f} ms  ({hi/lo if lo else float('nan'):.2f}x)")
            ideal = pl[0][1] / (pl[-1][0] / pl[0][0])
            print(f"      ⊘ if the cost were PURELY per-sync, K={pl[-1][0]} would read "
                  f"{ideal:.3f} ms/launch (1/K of K={pl[0][0]}'s). It reads {pl[-1][1]:.3f}.")
            print()
            r = fit_block([(K, med) for (K, med, *_) in rows], "ALL K")
            if r and len(rows) > 3:
                kmax = max(K for K, *_ in rows)
                print()
                fit_block([(K, med) for (K, med, *_) in rows if K != kmax],
                          f"DROP K={kmax} (is it linear?)")
            # the thread-state reading of the same rows
            print("\n    ★ THE THREAD STATE ACROSS THE SWEEP — is the wait ON-CPU or OFF-CPU?")
            for (K, med, cpu, off, csw, bad) in rows:
                share = 100.0 * off / med if med > 0 else float("nan")
                print(f"      K={K:<5d} offcpu={off:8.3f} ms ({share:5.1f} % of the batch) "
                      f"nvcsw={csw:.1f}")

    # ---- the per-iteration thread-state breakdown ---------------------------------------
    if iters:
        print("\n" + "=" * 78)
        print("★★★★★ THE SYNC BREAKDOWN — and it closes by construction")
        print("=" * 78)
        bysize = {}
        for t in iters:
            bysize.setdefault(int(t[0]), []).append(t)
        for N in sorted(bysize):
            rows = bysize[N][1:]  # ⊘ drop iteration 0: it carries publication + first touch
            if not rows:
                continue
            def med(f):
                v = sorted(f(r) for r in rows)
                return v[len(v) // 2]
            sync = med(lambda r: float(r[4]))
            cpu = med(lambda r: float(r[7]))
            off = med(lambda r: float(r[8]))
            sub = med(lambda r: float(r[3]))
            subc = med(lambda r: float(r[6]))
            csw = med(lambda r: float(r[9]))
            lau = med(lambda r: float(r[2]))
            print(f"\n  --- N={N}  (n={len(rows)} iterations, first discarded) ---")
            print(f"    launch_ms          {lau:9.3f}   100.0 %")
            print(f"    ├─ submit_ms       {sub:9.3f}   {100*sub/lau:5.1f} %   "
                  f"(cpu {subc:.3f})")
            print(f"    └─ sync_ms         {sync:9.3f}   {100*sync/lau:5.1f} %")
            print(f"       ├─ ON-CPU       {cpu:9.3f}   {100*cpu/sync:5.1f} % of sync")
            print(f"       └─ OFF-CPU      {off:9.3f}   {100*off/sync:5.1f} % of sync   "
                  f"(nvcsw {csw:.0f})")
            resid = sync - cpu - off
            print(f"       UNMARKED        {resid:9.6f}   "
                  f"{100*resid/sync if sync else 0:.4f} % of sync")
            print(f"       ⊘ UNMARKED is ~0 BY CONSTRUCTION (offcpu := wall - cpu). It is")
            print(f"         printed as an ARITHMETIC CHECK on the parser, not as a finding:")
            print(f"         a nonzero value here means these columns were misread, nothing more.")
            if cpu > off:
                print(f"    ⇒ the thread is RUNNING for {100*cpu/sync:.1f} % of the wait: it SPINS.")
                print(f"      A spin says the value it wants has not ARRIVED; it does not say")
                print(f"      the CPU is doing useful work, and it CANNOT name the producer.")
            else:
                print(f"    ⇒ the thread is OFF-CPU for {100*off/sync:.1f} % of the wait: it BLOCKS.")
                print(f"      nvcsw={csw:.0f} counts how many times. A block needs a WAKER, and")
                print(f"      naming the waker is a separate measurement this does not make.")

    # ---- sync vs size -------------------------------------------------------------------
    if len(set(int(t[0]) for t in iters)) >= 2:
        print("\n" + "=" * 78)
        print("★★★ SYNC vs KERNEL DURATION — the duration discriminator")
        print("=" * 78)
        bysize = {}
        for t in iters:
            bysize.setdefault(int(t[0]), []).append(float(t[4]))
        sizes = sorted(bysize)
        base = None
        print(f"    {'N':>6} {'flop/launch':>14} {'sync_med_ms':>12} {'x vs smallest':>14}")
        for N in sizes:
            v = sorted(bysize[N][1:] or bysize[N])
            m = v[len(v) // 2]
            if base is None:
                base, baseN = m, N
            fl = 2.0 * N ** 3
            print(f"    {N:>6} {fl:>14.3e} {m:>12.3f} {m/base:>13.2f}x")
        print(f"    ⊘ work scales as N^3: N={sizes[-1]} carries "
              f"{(sizes[-1]/sizes[0])**3:.0f}x the arithmetic of N={sizes[0]}.")
        print("    ★ THE KNOWN-POSITIVE: if the LARGEST size does not move sync, the duration")
        print("      knob is INERT and a flat curve at small N proves nothing. Check it first.")
    return 0


def selftest():
    """⊘ Offline, no GPU, no log. Checks the differ can DETECT, per the tree's own rule that
    a census zero needs a known-positive."""
    print("★ w320_fit selftest — the fit must RECOVER a planted line and REFUSE <3 points")
    ok = True
    a, b = ols([1, 2, 4, 8], [4 + 23.5, 8 + 23.5, 16 + 23.5, 32 + 23.5])
    if abs(a - 4.0) > 1e-9 or abs(b - 23.5) > 1e-9:
        print(f"    ✘ planted a=4 b=23.5, recovered a={a} b={b}")
        ok = False
    else:
        print(f"    ✔ planted (a=4, b=23.5) recovered exactly: a={a:.6f} b={b:.6f}")
    a2, b2 = ols([1, 2, 4, 8], [27.5, 55.0, 110.0, 220.0])
    if abs(a2 - 27.5) > 1e-9 or abs(b2) > 1e-9:
        print(f"    ✘ planted pure-slope, recovered a={a2} b={b2}")
        ok = False
    else:
        print(f"    ✔ planted pure per-launch (a=27.5, b=0) recovered: a={a2:.6f} b={b2:.6f}")
    if fit_block([(1, 10.0), (2, 20.0)], "TWO POINTS") is not None:
        print("    ✘ a 2-point fit was ACCEPTED — the refusal is dead")
        ok = False
    else:
        print("    ✔ a 2-point fit is REFUSED")
    # ★ the regexes must match the exact strings cup8bench prints, or this reports 0 rows and
    #   an empty result reads as 'nothing happened' rather than 'nothing was parsed'.
    s = ("BSWEEPSUM N=128 K=8 reps=5 med_total_ms=55.123 per_launch_ms=6.890 min_ms=54.0 "
         "max_ms=57.0 med_cpu_ms=30.1 med_offcpu_ms=25.0 med_nvcsw=3.0 bad=0")
    i = ("ITER N=128 i=3 launch_ms=27.552 submit_ms=4.040 sync_ms=23.512 t0_mono_ms=1234.5 "
         "sub_cpu_ms=0.900 syn_cpu_ms=23.100 syn_offcpu_ms=0.412 syn_nvcsw=0 syn_nivcsw=1 "
         "syn_utime_ms=20.000 syn_stime_ms=3.000 verified=yes bad=0")
    for name, rx, txt in (("BSWEEPSUM", SWEEP_RE, s), ("ITER", ITER_RE, i),
                          ("BSWEEPSUM+GUEST_", SWEEP_RE, "GUEST_" + s)):
        if rx.search(txt):
            print(f"    ✔ {name} regex matches the literal cup8bench line")
        else:
            print(f"    ✘✘ {name} regex does NOT match — this analyser would report 0 rows")
            ok = False
    print("★ SELFTEST", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--selftest":
        sys.exit(selftest())
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(64)
    sys.exit(main(sys.argv[1:]))
