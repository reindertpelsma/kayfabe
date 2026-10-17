#!/usr/bin/env python3
"""ladder.py TAG... : per boot (tag) and API: Heaven fps, credit waits per frame, credit-wait duration histogram (p50/p99/max us, from the `cw` tick lines),
publishes with the private ring empty while the guest ring was >= 2 ahead (per frame), GP entries per frame, and the private ring size asserted from the process environment.
Window = Heaven benchmark window trimmed 8/6 s, as perframe.py."""
import re, sys, gzip, os, datetime
EV = os.environ.get("GAPEV", "/data/claude-gap/ev")
def ticks(tag):
    d = {}
    for l in gzip.open(f"{EV}/qemulog/{tag}.qemu.log.gz", "rt", errors="replace"):
        m = re.search(r"LIFECYCLE t=[\d.]+ wall=([\d.]+) tick (.*)$", l)
        if not m: continue
        w, rest = float(m.group(1)), m.group(2)
        e = d.setdefault(w, {})
        if rest.startswith("cw "):
            h = eval(re.search(r"credit_wait_us_log2=(\[[^\]]*\])", rest).group(1)); e["hist"] = h
            for k in ("credit_wait_sum_us", "credit_wait_max_us", "publishes", "publishes_ring_empty_guest_ahead"): e[k] = int(re.search(k + r"=(\d+)", rest).group(1))
        elif rest.startswith("gp "): e["gp_entries"] = int(re.search(r"gp_entries=(\d+)", rest).group(1))
        elif rest.startswith("relay_stats"): e["credit_waits"] = int(re.search(r"credit_waits=(\d+)", rest).group(1)); e["spin"] = int(re.search(r"spin=(\d+)", rest).group(1)); e["backstop"] = int(re.search(r"backstop=(\d+)\)", rest).group(1))
    return d
def heaven(label, day):
    t = open(f"{EV}/heaven/{label}.out", errors="replace").read().replace("&nbsp;", " ")
    mr = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark running", t); me = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark results", t)
    tt = re.sub(r"\d\d:\d\d:\d\d", " ", t.replace("\n", " ")); m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+)", tt)
    if not (mr and me and m): return None
    mk = lambda x: datetime.datetime(day.year, day.month, day.day, *map(int, x.groups()), tzinfo=datetime.timezone.utc).timestamp()
    return mk(mr) + 8, mk(me) - 6, float(m.group(2))
def pct(h, q):
    n = sum(h)
    if not n: return 0
    c = 0
    for i, x in enumerate(h):
        c += x
        if c >= q * n: return 2 ** i   # lower edge of the log2 bucket, us
    return 2 ** (len(h) - 1)
print("%-8s %-6s %7s %9s %9s %10s %8s %8s %8s %8s %12s" % ("tag", "api", "fps", "gp/frame", "cw/frame", "starve/fr", "cw_p50us", "cw_p99us", "cw_maxus", "spin/fr", "ring(env)"))
for tag in sys.argv[1:]:
    try: d = ticks(tag)
    except FileNotFoundError: print(tag, "no log"); continue
    ws = sorted(d); day = datetime.datetime.fromtimestamp(ws[0], datetime.timezone.utc)
    ring = "?"
    try:
        a = open(f"{EV}/heaven/{tag.lower()}_d3d11/assert.txt").read(); m = re.search(r"KF3_PRIV_RING_ENTRIES=(\d+)", a); ring = m.group(1) if m else "512"
    except Exception: pass
    for api in ("d3d11", "gl"):
        try: h = heaven(f"{tag.lower()}_{api}", day)
        except FileNotFoundError: h = None
        if not h: continue
        at = lambda w: ([x for x in ws if x <= w and "hist" in d[x] and "gp_entries" in d[x]] or [ws[0]])[-1]
        a, b = at(h[0]), at(h[1]); fr = h[2] * (b - a)
        if fr <= 0:
            print("%-8s %-6s window has no usable tick dump pair (skipped)" % (tag, api)); continue
        A, B = d[a], d[b]
        diff = lambda k: B.get(k, 0) - A.get(k, 0)
        hist = [B["hist"][i] - A["hist"][i] for i in range(len(B["hist"]))] if "hist" in A and "hist" in B else [0] * 24
        print("%-8s %-6s %7.2f %9.1f %9.3f %10.4f %8d %8d %8d %8.1f %12s" % (tag, api, h[2], diff("gp_entries") / fr, diff("credit_waits") / fr, diff("publishes_ring_empty_guest_ahead") / fr, pct(hist, .5), pct(hist, .99), B.get("credit_wait_max_us", 0), diff("spin") / fr, ring))
