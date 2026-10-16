#!/usr/bin/env python3
"""setobj_sum.py TAG : summarise the `LIFECYCLE_SETOBJ` lines of a scan-on control boot per workload phase (boot, Heaven D3D11, Heaven OpenGL, Edge).
A channel belongs to the phase in which it was FIRST seen (first line wall time). Per phase: channels, max set_object_total per channel, sum of rebinds
(after-first), same-resend / different, extra-subchannel 5080 binds, late SET_OBJECTs (after the channel's 20th GP entry) with their entry indexes, distinct data words."""
import re, sys, gzip, os, datetime, collections
EV = os.environ.get("GAPEV", "/data/claude-gap/ev"); tag = sys.argv[1]
ch = {}
first = {}
for l in gzip.open(f"{EV}/qemulog/{tag}.qemu.log.gz", "rt", errors="replace"):
    m = re.search(r"LIFECYCLE_SETOBJ t=[\d.]+ wall=([\d.]+) (\w+) (\S+) (.*)$", l)
    if not m: continue
    w, k, rest = float(m.group(1)), m.group(3), m.group(4)
    first.setdefault(k, w)
    d = {}
    for name in ("total", "same_resend", "different", "5080_extra_sub", "late_first_idx", "late_count"):
        d[name] = int(re.search(r"set_object_%s=(-?\d+)" % name, rest).group(1))
    for name in ("per_sub", "after_first_per_sub", "late_idx", "distinct"):
        d[name] = re.search(r"set_object_%s=(\[[^\]]*\])" % name, rest).group(1)
    ch[k] = d
day = datetime.datetime.fromtimestamp(min(first.values()), datetime.timezone.utc)
def win(label):
    try: t = open(f"{EV}/heaven/{label}.out", errors="replace").read().replace("&nbsp;", " ")
    except FileNotFoundError: return None
    a = re.search(r"(\d\d):(\d\d):(\d\d) LAUNCH|LAUNCH (\d\d):(\d\d):(\d\d)", t)
    mr = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark running", t); me = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark results", t)
    mk = lambda x: datetime.datetime(day.year, day.month, day.day, *map(int, x.groups()), tzinfo=datetime.timezone.utc).timestamp()
    return (mk(mr) - 60, mk(me) + 10) if mr and me else None
wd, wg = win(f"{tag.lower()}_d3d11"), win(f"{tag.lower()}_gl")
edge_start = None
try:
    t = open(f"{EV}/edge_{tag}.out").read(); m = re.search(r"START (\d\d):(\d\d):(\d\d)", t)
    if m: edge_start = datetime.datetime(day.year, day.month, day.day, *map(int, m.groups()), tzinfo=datetime.timezone.utc).timestamp()
except FileNotFoundError: pass
def phase(w):
    if wd and wd[0] <= w <= wd[1]: return "heaven_d3d11"
    if wg and wg[0] <= w <= wg[1]: return "heaven_opengl"
    if edge_start and edge_start - 5 <= w <= edge_start + 75: return "edge"
    return "other (boot/idle/between)"
ph = collections.defaultdict(list)
for k, w in first.items(): ph[phase(w)].append(k)
print(f"# {tag}: {len(ch)} channels with at least one SET_OBJECT")
for p, ks in sorted(ph.items()):
    ds = [ch[k] for k in ks]
    rebinds = sum(sum(eval(d["after_first_per_sub"])) for d in ds)
    late = [(k, ch[k]["late_count"], ch[k]["late_idx"]) for k in ks if ch[k]["late_count"] > 0]
    distinct = sorted({x for d in ds for x in [int(y, 16) for y in re.findall(r"[0-9a-f]+", d["distinct"])]})
    print(f"== {p}: channels={len(ks)} max_set_object_total_per_channel={max(d['total'] for d in ds)} rebinds_total={rebinds} same_resend={sum(d['same_resend'] for d in ds)} different={sum(d['different'] for d in ds)} extra_sub_5080={sum(d['5080_extra_sub'] for d in ds)} channels_with_late={len(late)}")
    print("   distinct data words (hex):", [hex(x) for x in distinct])
    for k, n, idx in late[:10]: print(f"   late: channel {k} count={n} entry_idx={idx}")
