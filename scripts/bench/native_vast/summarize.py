#!/usr/bin/env python3
"""summarize.py RUNSDIR [--json out.json] : per-run metrics and per-API medians for native runs (RUNSDIR/<label>/dmon.txt + events.txt), warm-up excluded.
Heaven's FPS is parsed from events.txt (HLOG lines; the engine glues its timestamp to each number, so the timestamp is used as the delimiter)."""
import glob, json, os, re, statistics as st, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from analyze import analyze

def heaven(ev):
    t = open(ev, errors="replace").read().replace("\r", "")
    ts = re.search(r"HLOG (\d\d:\d\d:\d\d) Benchmark results", t)
    if not ts:
        return None
    s = ts.group(1)
    g = lambda pat: float(re.search(pat + r"([\d.]+?)" + s, t).group(1))
    return dict(fps=g(r"(?<!Min )(?<!Max )FPS:\s+"), min_fps=g(r"Min FPS:\s+"), max_fps=g(r"Max FPS:\s+"), frames=int(g(r"Frames:\s+")), time_s=g(r"Time:\s+"), score=float(re.search(r"Score:\s+([\d.]+)", t).group(1)))

def runs(d):
    out = []
    for dm in sorted(glob.glob(os.path.join(d, "*", "dmon.txt"))):
        lab = os.path.basename(os.path.dirname(dm))
        h = heaven(os.path.join(os.path.dirname(dm), "events.txt"))
        if not h:
            continue
        r = analyze(dm, h["fps"], lab); r.update({k: h[k] for k in ("min_fps", "max_fps", "frames", "time_s", "score")}); r["api"] = "d3d11" if lab.startswith(("d", "warm")) else "gl"
        out.append(r)
    return out

def med(rows, k):
    x = [r[k] for r in rows if r.get(k) == r.get(k)]
    return st.median(x) if x else float("nan")

KEYS = ["fps", "power_w", "sm_pct", "mem_pct", "sm_mhz_med", "mem_mhz_med", "pcie_rx_mbs", "pcie_tx_mbs", "j_per_frame", "gpu_ms_per_frame", "frame_ms", "pcie_mb_per_frame", "w_per_sm_pct", "fps_per_w", "temp_c_med"]

if __name__ == "__main__":
    rs = runs(sys.argv[1]); res = {"runs": rs}
    for api in ("d3d11", "gl"):
        sel = [r for r in rs if r["api"] == api and r["label"] != "warm"]
        res[api] = {k: round(med(sel, k), 4) for k in KEYS}; res[api]["n"] = len(sel)
    d, g = res["d3d11"], res["gl"]
    res["ratios_d3d11_over_gl"] = {"r_fps": d["fps"] / g["fps"], "r_E": d["j_per_frame"] / g["j_per_frame"], "r_G": d["gpu_ms_per_frame"] / g["gpu_ms_per_frame"],
                                   "r_P": d["pcie_mb_per_frame"] / g["pcie_mb_per_frame"], "r_W": d["w_per_sm_pct"] / g["w_per_sm_pct"]}
    for r in rs:
        print("%-5s %-5s fps %7.2f min %6.2f max %7.2f P %6.1fW sm%% %5.1f mem%% %5.1f smMHz %6.0f memMHz %6.0f rx %6.0f tx %5.0f MB/s  J/f %.3f  gpu_ms/f %.2f  pcie_MB/f %.2f  W/sm%% %.2f" % (
            r["label"], r["api"], r["fps"], r["min_fps"], r["max_fps"], r["power_w"], r["sm_pct"], r["mem_pct"], r["sm_mhz_med"], r["mem_mhz_med"], r["pcie_rx_mbs"], r["pcie_tx_mbs"], r["j_per_frame"], r["gpu_ms_per_frame"], r["pcie_mb_per_frame"], r["w_per_sm_pct"]))
    for api in ("d3d11", "gl"):
        print("MEDIAN", api, json.dumps(res[api]))
    print("RATIOS D3D11/GL", json.dumps({k: round(v, 3) for k, v in res["ratios_d3d11_over_gl"].items()}))
    if "--json" in sys.argv:
        json.dump(res, open(sys.argv[sys.argv.index("--json") + 1], "w"), indent=1)
