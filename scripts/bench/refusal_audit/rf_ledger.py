#!/usr/bin/env python3
"""rf_ledger.py <rundir> [--ogkm DIR] — per-workload KERNEL-SIDE refusals from the kf3 ledger.

Each workload dir holds kf3.log (the device's stderr slice for that workload) and
kf3_ledger_before.log (the heartbeat in force when it started). The heartbeat carries
`gsp_refusals[total=N distinct=M [fn76/0x2080xxxx=0x56xK ...]]` (kf-gsp's RefusalLedger, cumulative
per QEMU run), so the rows this workload caused = (last heartbeat in the slice) - (the one before).
Prints one line per workload and a matrix of (row -> workloads). ⊘ A workload whose slice has no
heartbeat is UNMEASURED, not clean.
"""
import glob, os, re, sys, json
ROW = re.compile(r"fn(\d+)(?:/0x([0-9a-f]+))?=0x([0-9a-f]+)x(\d+)")

def ledger(line):
    m = re.search(r"gsp_refusals\[(.*)\]\s*$", line)
    if not m:
        return None
    return {(int(r.group(1)), r.group(2), r.group(3)): int(r.group(4)) for r in ROW.finditer(m.group(1))}

def last_ledger(path):
    best = None
    try:
        for l in open(path, errors="replace"):
            if "gsp_refusals[" in l:
                best = ledger(l) or best
    except OSError:
        pass
    return best

def main():
    run = sys.argv[1]
    ogkm = sys.argv[sys.argv.index("--ogkm") + 1] if "--ogkm" in sys.argv else None
    names = {}
    if ogkm:
        sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "../.."))
        import nvdiff_status as ns
        names = ns.load_names(ogkm)
    matrix = {}
    for d in sorted(glob.glob(os.path.join(run, "*", "kf3.log"))):
        w = os.path.basename(os.path.dirname(d))
        before = last_ledger(os.path.join(os.path.dirname(d), "kf3_ledger_before.log")) or {}
        after = last_ledger(d)
        if after is None:
            print("%-18s UNMEASURED (no heartbeat in the slice)" % w)
            continue
        delta = {k: v - before.get(k, 0) for k, v in after.items() if v - before.get(k, 0) > 0}
        print("%-18s %d refusals, %d rows" % (w, sum(delta.values()), len(delta)))
        for k, n in delta.items():
            matrix.setdefault(k, {})[w] = n
    print("== kernel-side rows by workload")
    for (fn, det, st), ws in sorted(matrix.items(), key=lambda kv: -len(kv[1])):
        nm = names.get(int(det, 16), "?") if (det and fn == 76) else ("class 0x%s" % det if det else "")
        print("  fn%-3d %-10s st=0x%-3s %-58s %s" % (fn, "0x" + det if det else "-", st, nm[:58],
              " ".join("%s:%d" % kv for kv in sorted(ws.items()))))
    if "--json" in sys.argv:
        json.dump([{"fn": fn, "detail": det, "status": st, "workloads": ws} for (fn, det, st), ws in matrix.items()],
                  open(sys.argv[sys.argv.index("--json") + 1], "w"), indent=1)

if __name__ == "__main__":
    main()
