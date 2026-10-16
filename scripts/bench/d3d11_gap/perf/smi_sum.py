import sys, statistics as st
for path in sys.argv[1:]:
    t = open(path, errors="replace").read()
    if "SMI_CSV_BEGIN" not in t: print(path.split("/")[-1], "no smi"); continue
    rows = [l.strip().split(",") for l in t.split("SMI_CSV_BEGIN")[1].split("SMI_CSV_END")[0].splitlines() if l and not l.startswith("t,")]
    def f(i):
        out = []
        for r in rows:
            try: out.append(float(r[i]))
            except Exception: out.append(float("nan"))
        return out
    u = f(1); idx = [i for i, x in enumerate(u) if x == x and x > 20]
    if not idx: print(path.split("/")[-1], "samples", len(rows), "no busy"); continue
    g = lambda c: [f(c)[i] for i in idx]
    print(path.split("/")[-1], "samples", len(rows), "busy", len(idx), "util avg", round(st.mean(g(1)), 1), "sm_mhz med", st.median(g(2)), "mem_mhz", st.median(g(3)), "power avg", round(st.mean(g(4)), 1), "pstate", sorted(set(rows[i][5] for i in idx)), "temp", st.median(g(6)))
