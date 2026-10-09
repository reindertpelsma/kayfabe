#!/usr/bin/env python3
"""Offline test of ddsearch with a simulated driver: needs {A} or {B and C} to be refused to die."""
import os, sys, tempfile, subprocess, json
d = tempfile.mkdtemp()
lst = os.path.join(d, "l.txt")
keys = [0x20800000 + i for i in range(40)]
open(lst, "w").write("# t\n" + "".join("76 %#x  # r\n" % k for k in keys))
orc = os.path.join(d, "oracle.py")
open(orc, "w").write("""import sys
t=open(sys.argv[1]).read()
has=lambda k: ('%#x'%k) in t
dead = has(0x20800007) or (has(0x20800010) and has(0x20800021))
sys.exit(1 if dead else 0)
""")
r = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "ddsearch.py"), "--list", lst,
                    "--oracle", "%s %s {subset} {out}" % (sys.executable, orc), "--work", d + "/w", "--reps", "1"],
                   capture_output=True, text=True)
res = json.load(open(d + "/w/result.json"))
sets = sorted(sorted(x[1] for x in s["rules"]) for s in res["culprit_sets"])
assert sets == [["0x20800007"], ["0x20800010", "0x20800021"]], sets
print("ok: found", sets, "in", res["boots"], "simulated boots of 40 rules")
