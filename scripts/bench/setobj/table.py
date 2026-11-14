#!/usr/bin/env python3
"""table.py DIR... : one compact line per (case, rep): step verdicts, the final settle tail, reach results and the
journal Xid lines for the probe's pid. Used to build REPORT.txt."""
import os, re, sys

def short(x):
    return x.replace("sem_arrived=", "sem=").replace("notifier_error=", "ne=")

def main(dirs):
    rows = []
    for d in dirs:
        for n in sorted(os.listdir(d)):
            if not n.endswith(".log"):
                continue
            b = n[:-4]
            log = open(os.path.join(d, n), errors="replace").read().splitlines()
            meta = open(os.path.join(d, b + ".meta")).read() if os.path.exists(os.path.join(d, b + ".meta")) else ""
            pid = (re.search(r"pid=(\d+)", meta) or [0, "?"])[1]
            jl = open(os.path.join(d, b + ".journal"), errors="replace").read().splitlines() if os.path.exists(os.path.join(d, b + ".journal")) else []
            xid = [re.sub(r"^(\S+) claude kernel: NVRM: ", r"\1 ", x) for x in jl if f"pid={pid}," in x]
            out = [f"{d}/{b}"]
            for ln in log:
                m = re.match(r"RES t=\S+ e=\S+ step=(.*?) sem_arrived=(\d) notifier_error=(\d)", ln)
                if m:
                    out.append(f"  step[{m.group(1)}] sem={m.group(2)} ne={m.group(3)}")
                m = re.match(r"RES t=\S+ e=\S+ step=(.*?) \+250ms notifier_error=(\d)", ln)
                if m:
                    out.append(f"      +250ms ne={m.group(2)}")
                if re.search(r"RES .*(alloc_sw|alloc_sw .*FAILED|executed_by|ORACLE|engine_sem|register H)", ln) or ("RES" in ln and "REJECTED" in ln):
                    out.append("  " + re.sub(r"^RES t=\S+ e=\S+ ", "", ln))
                if re.search(r"RM .*(free |alloc [AB] .*status=0x[0-9a-f]*[1-9a-f]|remove_api|control register|control A |control B |compute alloc|3d alloc)", ln) and "status=0x" in ln:
                    out.append("  " + re.sub(r"^RM t=\S+ e=\S+ ", "", ln))
                if "SETUP-FAIL" in ln or "FAILED" in ln:
                    out.append("  !! " + ln[:200])
            out.append(f"  journal(pid {pid}): " + (" | ".join(xid) if xid else "none"))
            rows.append("\n".join(out))
    print("\n".join(rows))

main(sys.argv[1:])
