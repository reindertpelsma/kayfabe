#!/usr/bin/env python3
"""runtab.py LABEL... : one line per Heaven run: fps, host dmon means in the benchmark window (power, util, sm/mem clock, pcie rx/tx, pviol), vmstat steal/interrupts,
memory PSI deltas, kf3 worker CPU, broker backend. EV dir /data/claude-gap/ev (GAPEV overrides). dmon columns are read from the header."""
import re, sys, os, datetime, statistics as st
EV = os.environ.get("GAPEV", "/data/claude-gap/ev"); H = EV + "/heaven"
def hms(m): h, mi, s = map(int, m.groups()); return h * 3600 + mi * 60 + s
def tz_off():
    return int(datetime.datetime.now().astimezone().utcoffset().total_seconds())
def parse(label):
    t = open(f"{H}/{label}.out", errors="replace").read().replace("&nbsp;", " ")
    tt = re.sub(r"\d\d:\d\d:\d\d", " ", t.replace("\n", " "))
    m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+)\D+?Min FPS:\s+([\d.]+).*?Max FPS:\s+([\d.]+)", tt)
    if not m: return None
    o = dict(label=label, frames=int(m.group(1)), fps=float(m.group(2)), minfps=float(m.group(3)))
    mr = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark running", t); me = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark results", t)
    if mr and me:
        a, b = (hms(mr) + tz_off() + 8) % 86400, (hms(me) + tz_off() - 6) % 86400; o["win"] = (a, b)
        try:
            ls = open(f"{H}/{label}/host_dmon.txt").read().splitlines()
            hdr = [l for l in ls if l.startswith("#") and "gpu" in l][0].lstrip("#").split()
            idx = {n: i for i, n in enumerate(hdr)}
            rows = []
            for l in ls:
                f = l.split()
                if f and re.match(r"\d\d:\d\d:\d\d", f[0]):
                    h, mi, s = map(int, f[0].split(":"))
                    if a <= h * 3600 + mi * 60 + s <= b: rows.append(f)
            def col(n):
                if n not in idx: return None
                v = [float(r[idx[n]]) for r in rows if r[idx[n]] not in ("-",)]
                return st.mean(v) if v else None
            for n in ("pwr", "sm", "mem", "mclk", "pclk", "pviol", "rxpci", "txpci", "fb", "bar1"): o[n] = col(n)
            o["dmon_n"] = len(rows)
        except Exception as e: o["dmon_err"] = str(e)[:40]
        try:
            vs = open(f"{H}/{label}/vmstat.txt").read().splitlines()
            hdr = [l for l in vs if " r " in l and "swpd" in l][0].split()
            rows = [l.split() for l in vs if re.match(r"\s*\d", l)]
            rows = rows[10:-6] if len(rows) > 40 else rows
            col = lambda n: st.mean(float(r[hdr.index(n)]) for r in rows)
            o["vm_in_k"] = col("in") / 1000; o["vm_st"] = col("st"); o["vm_si"] = col("si"); o["vm_so"] = col("so")
        except Exception as e: pass
    try:
        for l in open(f"{H}/{label}/threads_cpu.txt"):
            f = l.split()
            if len(f) >= 4 and f[1].startswith("kf3-worker"): o[f[1]] = float(f[2]) + float(f[3])
    except FileNotFoundError: pass
    def psi(p):
        d = {}
        try:
            for l in open(p):
                m = re.search(r"MEMREC \S+ psi.memory: (some|full) avg10=([\d.]+) avg60=([\d.]+) avg300=([\d.]+) total=(\d+)", l)
                if m: d[m.group(1)] = (float(m.group(3)), int(m.group(5)))
                m = re.search(r"MEMREC \S+ vmstat: (.*)", l)
                if m: d["vm"] = dict((k, int(v)) for k, v in re.findall(r"(\w+) (\d+)", m.group(1)))
                m = re.search(r"MEMREC \S+ meminfo: (.*)", l)
                if m: d["mi"] = dict((k, int(v)) for k, v in re.findall(r"(\w+):\s+(\d+)", m.group(1)))
        except FileNotFoundError: pass
        return d
    a, b = psi(f"{H}/{label}/env_pre.txt"), psi(f"{H}/{label}/env_post.txt")
    if a and b:
        o["psi_full_ms"] = (b["full"][1] - a["full"][1]) / 1000; o["psi_some_ms"] = (b["some"][1] - a["some"][1]) / 1000
        o["majflt"] = b["vm"]["pgmajfault"] - a["vm"]["pgmajfault"]; o["refault"] = b["vm"]["workingset_refault_file"] - a["vm"]["workingset_refault_file"]
        o["pswpin"] = b["vm"]["pswpin"] - a["vm"]["pswpin"]; o["swap_used_mb_pre"] = (a["mi"]["SwapTotal"] - a["mi"]["SwapFree"]) / 1024
    try:
        o["broker"] = re.search(r"broker.*?(\w+) .*--backend (\w+)", open(f"{H}/{label}/assert.txt").read().replace("ASSERT ", "")).groups()
    except Exception: pass
    return o
f = lambda x, p="%.0f": (p % x) if x is not None else "-"
print("%-14s %7s %6s | %5s %4s %4s %5s %5s %6s %6s %5s | %6s %5s %5s | %5s %6s %6s %6s %7s %7s | broker" % ("label", "fps", "min", "pwrW", "sm%", "mem%", "smMHz", "memMHz", "rxMB/s", "txMB/s", "pviol", "w0cpu", "w1cpu", "", "in_k", "steal", "swpin", "psiF_ms", "majflt", "refault"))
for lab in sys.argv[1:]:
    o = parse(lab)
    if not o: print(lab, "no result"); continue
    print("%-14s %7.2f %6.1f | %5s %4s %4s %5s %5s %6s %6s %5s | %6s %5s %5s | %5s %6s %6s %6s %7s %7s | %s" % (lab, o["fps"], o["minfps"], f(o.get("pwr")), f(o.get("sm")), f(o.get("mem")), f(o.get("pclk")), f(o.get("mclk")), f(o.get("rxpci")), f(o.get("txpci")), f(o.get("pviol"), "%.1f"),
        f(o.get("kf3-worker0")), f(o.get("kf3-worker1")), "", f(o.get("vm_in_k")), f(o.get("vm_st"), "%.1f"), f(o.get("pswpin")), f(o.get("psi_full_ms")), f(o.get("majflt")), f(o.get("refault")), o.get("broker", "-")))
