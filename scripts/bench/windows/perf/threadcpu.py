#!/usr/bin/env python3
"""threadcpu.py snap PID OUT | diff A B : per-thread utime/stime (clock ticks) and context switches from /proc/PID/task/*; diff prints per-thread user/sys seconds and ctxt switches."""
import sys, os, re, json
def snap(pid):
    d = {}
    for t in os.listdir("/proc/%s/task" % pid):
        try:
            s = open("/proc/%s/task/%s/stat" % (pid, t)).read()
            m = re.match(r"(\d+) \((.*)\) (\S) (.*)", s, re.S); f = m.group(4).split()
            st = open("/proc/%s/task/%s/status" % (pid, t)).read()
            vol = int(re.search(r"voluntary_ctxt_switches:\s+(\d+)", st).group(1)); inv = int(re.search(r"nonvoluntary_ctxt_switches:\s+(\d+)", st).group(1))
            d[t] = dict(comm=m.group(2), ut=int(f[10]), st=int(f[11]), vol=vol, inv=inv)
        except Exception: pass
    return d
if sys.argv[1] == "snap": json.dump(snap(sys.argv[2]), open(sys.argv[3], "w"))
else:
    a = json.load(open(sys.argv[2])); b = json.load(open(sys.argv[3])); hz = os.sysconf("SC_CLK_TCK"); rows = []
    for t, y in b.items():
        x = a.get(t, dict(ut=0, st=0, vol=0, inv=0)); rows.append((y["ut"] - x["ut"] + y["st"] - x["st"], t, y["comm"], (y["ut"] - x["ut"]) / hz, (y["st"] - x["st"]) / hz, y["vol"] - x["vol"], y["inv"] - x["inv"]))
    rows.sort(reverse=True)
    print("%8s %-18s %8s %8s %9s %9s" % ("tid", "comm", "user_s", "sys_s", "vol_csw", "invol_csw"))
    for r in rows[:int(sys.argv[4]) if len(sys.argv) > 4 else 14]: print("%8s %-18s %8.2f %8.2f %9d %9d" % (r[1], r[2][:18], r[3], r[4], r[5], r[6]))
