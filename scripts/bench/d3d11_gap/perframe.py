#!/usr/bin/env python3
"""perframe.py TAG [TRIM_START TRIM_END] : per-frame counters of one boot, Heaven D3D11 vs OpenGL side by side.
Reads /data/claude-gap/ev/qemulog/TAG.qemu.log.gz (gz by stopk.sh) and ev/heaven/<tag>_{d3d11,gl}.out. Windows come from Heaven's own
'Benchmark running' .. 'Benchmark results' lines (guest UTC), trimmed by TRIM_START/TRIM_END s. Every cumulative numeric field of the
`kf3: LIFECYCLE` tick lines (gp, loop, relay_stats, gap, read sites) is diffed between the last dump at/before window start and window end."""
import re, sys, gzip, datetime, os
EV = os.environ.get("GAPEV", "/data/claude-gap/ev")
tag = sys.argv[1]; T0 = float(sys.argv[2]) if len(sys.argv) > 2 else 8.0; T1 = float(sys.argv[3]) if len(sys.argv) > 3 else 6.0
def hms(h, m, s, day):
    return datetime.datetime(day.year, day.month, day.day, h, m, s, tzinfo=datetime.timezone.utc).timestamp()
def heaven(label, day):
    t = open(f"{EV}/heaven/{label}.out", errors="replace").read().replace("&nbsp;", " ")
    mr = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark running", t); me = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark results", t)
    tt = re.sub(r"\d\d:\d\d:\d\d", " ", t.replace("\n", " "))
    m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+)", tt)
    if not (mr and me and m): return None
    return hms(*map(int, mr.groups()), day), hms(*map(int, me.groups()), day), int(m.group(1)), float(m.group(2))
num = re.compile(r"([A-Za-z_][\w.]*)=(-?\d+(?:\.\d+)?)(?![\w.])")
def flat(rest):
    d = {}
    # relay_stats: wakes(doorbell=1 event=2 ...) backstop_found(work=..) -> prefixed
    for grp in re.finditer(r"(\w+)\(([^)]*)\)", rest):
        for k, v in re.findall(r"(\w+)=(-?\d+)", grp.group(2)): d[f"{grp.group(1)}.{k}"] = float(v)
    # engines=[name:k=v,k=v name2:...]
    for em in re.finditer(r"(\w+):((?:\w+=-?\d+,?)+)", rest):
        for k, v in re.findall(r"(\w+)=(-?\d+)", em.group(2)): d[f"eng.{em.group(1)}.{k}"] = float(v)
    for k, v in num.findall(rest): d.setdefault(k, float(v))
    return d
dumps = {}
op = gzip.open if os.path.exists(f"{EV}/qemulog/{tag}.qemu.log.gz") else open
path = f"{EV}/qemulog/{tag}.qemu.log.gz" if op is gzip.open else f"{EV}/qemulog/{tag}.qemu.log"
for line in op(path, "rt", errors="replace"):
    if "kf3: LIFECYCLE " not in line: continue
    m = re.search(r"LIFECYCLE t=([\d.]+) wall=([\d.]+) (\w+) (.*)$", line)
    if not m or m.group(3) != "tick": continue
    wall, rest = float(m.group(2)), m.group(4)
    kind = rest.split()[0] if rest.split()[0] in ("gp", "loop", "gap", "relay_stats") else None
    if rest.startswith("read "):
        f = dict(re.findall(r"(\w+)=(\S+)", rest[5:])); pre = f"read.{f['site']}.{f['class']}."
        d = {pre + k: float(v) for k, v in f.items() if re.fullmatch(r"-?\d+", v)}
    elif kind: d = {f"{kind}.{k}": v for k, v in flat(rest).items()}
    else: continue
    dumps.setdefault(wall, {}).update(d)
walls = sorted(dumps)
if not walls: print("no tick dumps"); sys.exit(1)
day = datetime.datetime.fromtimestamp(walls[0], datetime.timezone.utc)
res = {}
for api, lab in (("d3d11", f"{tag.lower()}_d3d11"), ("gl", f"{tag.lower()}_gl")):
    try: h = heaven(lab, day)
    except FileNotFoundError: h = None
    if not h: continue
    a_t, b_t = h[0] + T0, h[1] - T1
    at = lambda w: ([x for x in walls if x <= w] or [walls[0]])[-1]
    a, b = at(a_t), at(b_t); dt = b - a; frames = h[3] * dt
    # fields cumulative at b minus at a (carry forward the latest value per key at or before each dump)
    def state(w):
        s = {}
        for x in walls:
            if x > w: break
            s.update(dumps[x])
        return s
    A, B = state(a), state(b)
    res[api] = dict(dt=dt, frames=frames, fps=h[3], d={k: (B[k] - A.get(k, 0)) for k in B})
if not res: print("no heaven results for", tag); sys.exit(1)
print(f"# {tag}: " + "; ".join(f"{k}: fps={v['fps']:.1f} window={v['dt']:.0f}s frames~{v['frames']:.0f}" for k, v in res.items()))
keys = sorted(set().union(*[set(v["d"]) for v in res.values()]))
print("%-52s %14s %14s %8s" % ("counter (per frame)", "d3d11", "gl", "d3d11/gl"))
for k in keys:
    vals = {api: res[api]["d"].get(k, 0) / res[api]["frames"] for api in res}
    if all(abs(x) < 1e-9 for x in vals.values()): continue
    x, y = vals.get("d3d11"), vals.get("gl")
    r = ("%.2f" % (x / y)) if (x is not None and y) else "-"
    print("%-52s %14s %14s %8s" % (k, "%.3f" % x if x is not None else "-", "%.3f" % y if y is not None else "-", r))
