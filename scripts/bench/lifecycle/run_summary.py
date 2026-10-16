#!/usr/bin/env python3
"""run_summary.py LABEL... : one line per Heaven run under /data/claude-perf2/ev/heaven/LABEL(.out): fps, min, max,
host dmon means inside the benchmark window (GPU util %, power W, SM clock, mem clock), kf3 worker CPU (user+sys s) and the
host Xid delta / guest events recorded by hv2.sh. Window = Heaven's own 'Benchmark running' .. 'Benchmark results'."""
import re, sys, os, datetime, statistics as st
EV = "/data/claude-perf2/ev/heaven"
def parse(label):
    out = {}
    t = open(f"{EV}/{label}.out", errors="replace").read().replace("&nbsp;", " ")
    tt = re.sub(r"\d\d:\d\d:\d\d", " ", t.replace("\n", " "))
    m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+)\D+?Min FPS:\s+([\d.]+).*?Max FPS:\s+([\d.]+)", tt)
    if not m:
        return {"label": label, "fps": None}
    out.update(label=label, frames=int(m.group(1)), fps=float(m.group(2)), minfps=float(m.group(3)), maxfps=float(m.group(4)))
    mr = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark running", t); me = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark results", t)
    # dmon rows are host-local time (CEST = UTC+2 in this campaign): compare as HH:MM:SS strings after converting guest UTC
    def tolocal(m):
        h, mi, s = map(int, m.groups()); return (h * 3600 + mi * 60 + s + 7200) % 86400
    if mr and me:
        a, b = tolocal(mr) + 8, tolocal(me) - 6
        rows = []
        try:
            for l in open(f"{EV}/{label}/host_dmon.txt"):
                f = l.split()
                if len(f) >= 15 and re.match(r"\d\d:\d\d:\d\d", f[0]):
                    h, mi, s = map(int, f[0].split(":")); sec = h * 3600 + mi * 60 + s
                    if a <= sec <= b:
                        rows.append(f)
        except FileNotFoundError:
            pass
        if rows:
            col = lambda i: [float(r[i]) for r in rows if r[i] != "-"]
            out.update(gpu_util=st.mean(col(5)), power=st.mean(col(2)), mclk=st.mean(col(11)), pclk=st.mean(col(12)), dmon_n=len(rows))
    try:
        for l in open(f"{EV}/{label}/threads_cpu.txt"):
            f = l.split()
            if len(f) >= 4 and f[1].startswith("kf3-worker"):
                out[f[1]] = float(f[2]) + float(f[3])
    except FileNotFoundError:
        pass
    try:
        sm = open(f"{EV}/{label}/startmarker").read()
        out["marker"] = " ".join(l for l in sm.splitlines() if l.startswith("events:"))
    except FileNotFoundError:
        pass
    return out
for lab in sys.argv[1:]:
    r = parse(lab)
    if r.get("fps") is None:
        print(f"{lab}: no result"); continue
    print("{label:22s} fps={fps:8.2f} min={minfps:7.2f} max={maxfps:7.2f} frames={frames} gpu_util={gu} pwr={pw} sm_mhz={pc} mem_mhz={mc} w0_cpu_s={w0} w1_cpu_s={w1} {marker}".format(
        gu="%.1f" % r["gpu_util"] if "gpu_util" in r else "-", pw="%.0f" % r["power"] if "power" in r else "-",
        pc="%.0f" % r["pclk"] if "pclk" in r else "-", mc="%.0f" % r["mclk"] if "mclk" in r else "-",
        w0="%.0f" % r.get("kf3-worker0", 0), w1="%.0f" % r.get("kf3-worker1", 0), marker=r.get("marker", ""), **{k: r[k] for k in ("label", "fps", "minfps", "maxfps", "frames")}))
