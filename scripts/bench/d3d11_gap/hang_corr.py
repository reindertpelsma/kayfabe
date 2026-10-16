#!/usr/bin/env python3
"""hang_corr.py TAG : coordinator item (2026-10-16): correlate the guest flip rate (`kms=` of the timestamped qemu.log `display fps` lines, ev/arm/TAG.dispts.txt)
with the 1 s host `nvidia-smi dmon` rows of the same second, inside each Heaven benchmark window of the arm (D3D11 and OpenGL).
Classes of display samples: stall (kms < 10), slow (10 <= kms < 40), normal (kms >= 40; the head is armed at 60 Hz). For each class: count, mean GPU sm% and power.
A stall sample with GPU sm% < 20 = 'guest paused with idle GPU'; with sm% >= 50 = 'GPU busy but not presenting'. Text output."""
import re, sys, os, datetime, statistics as st, time
EV = os.environ.get("GAPEV", "/data/claude-gap/ev"); tag = sys.argv[1]
def heaven_window(label):
    t = open(f"{EV}/heaven/{label}.out", errors="replace").read().replace("&nbsp;", " ")
    mr = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark running", t); me = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark results", t)
    if not (mr and me): return None
    f9 = float(open(f"{EV}/heaven/{label}/f9_epoch").read())
    d = datetime.datetime.fromtimestamp(f9, datetime.timezone.utc)
    mk = lambda m: datetime.datetime(d.year, d.month, d.day, *map(int, m.groups()), tzinfo=datetime.timezone.utc).timestamp()
    return mk(mr) + 8, mk(me) - 6
disp = []
for l in open(f"{EV}/arm/{tag}.dispts.txt", errors="replace"):
    m = re.match(r"(\d+\.\d+) .*kms=([\d.]+).*copies=([\d.]+)", l)
    if m: disp.append((float(m.group(1)), float(m.group(2)), float(m.group(3))))
print(f"# {tag}: {len(disp)} timestamped display lines" + (f", median interval {st.median([b[0]-a[0] for a,b in zip(disp,disp[1:])]):.2f} s" if len(disp) > 2 else ""))
for api in ("d3d11", "gl"):
    label = f"{tag.lower()}_{api}"
    try: w = heaven_window(label)
    except FileNotFoundError: w = None
    if not w: print(api, "no window"); continue
    dm = {}
    ls = open(f"{EV}/heaven/{label}/host_dmon.txt").read().splitlines()
    hdr = [l for l in ls if l.startswith("#") and "gpu" in l][0].lstrip("#").split(); ix = {n: i for i, n in enumerate(hdr)}
    day0 = datetime.datetime.fromtimestamp(w[0]).replace(hour=0, minute=0, second=0, microsecond=0).timestamp()
    for l in ls:
        f = l.split()
        if f and re.match(r"\d\d:\d\d:\d\d", f[0]):
            h, mi, s = map(int, f[0].split(":")); dm[int(day0 + h * 3600 + mi * 60 + s)] = (float(f[ix["sm"]]), float(f[ix["pwr"]]), float(f[ix["rxpci"]]) if f[ix["rxpci"]] != "-" else 0.0)
    cls = {"stall": [], "slow": [], "normal": []}
    for t, kms, cp in disp:
        if not (w[0] <= t <= w[1]): continue
        rows = [dm[s] for s in (int(t) - 1, int(t)) if s in dm]
        if not rows: continue
        c = "stall" if kms < 10 else "slow" if kms < 40 else "normal"
        cls[c].append((kms, st.mean(r[0] for r in rows), st.mean(r[1] for r in rows)))
    n = sum(len(v) for v in cls.values())
    print(f"== {api}: window {w[1]-w[0]:.0f} s, {n} display samples joined with dmon")
    for c, v in cls.items():
        if not v: print(f"  {c:7s} 0"); continue
        idle = sum(1 for x in v if x[1] < 20); busy = sum(1 for x in v if x[1] >= 50)
        print(f"  {c:7s} n={len(v):4d} ({100*len(v)/n:4.1f}%) kms_mean={st.mean(x[0] for x in v):5.1f} gpu_sm%_mean={st.mean(x[1] for x in v):5.1f} power_W_mean={st.mean(x[2] for x in v):5.1f} idle_gpu(<20%)={idle} busy_gpu(>=50%)={busy}")
