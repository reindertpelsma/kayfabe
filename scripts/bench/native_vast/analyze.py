#!/usr/bin/env python3
"""analyze.py dmon.txt FPS [--label L] : per-run metrics from an `nvidia-smi dmon` log and Heaven's FPS.
The busy window is the span from the first to the last 1 Hz sample with sm% >= 50, trimmed 3 s each end (the same rule for
native and kayfabe host logs, because the Heaven log and the dmon clock are on different machines for kayfabe).
Prints one JSON object. Quantities: mean power W, mean sm%, mean mem%, median clocks, mean pcie rx/tx MB/s, and per frame:
J/frame = W/fps, GPU time/frame (ms) = sm%/100/fps, PCIe MB/frame = (rx+tx)/fps, W per sm%."""
import json, statistics as st, sys

def parse(path):
    cols, rows = None, []
    for line in open(path, errors="replace"):
        if line.startswith("#"):
            t = line[1:].split()
            if cols is None and "gpu" in t:
                cols = t
            continue
        t = line.split()
        if cols is None or len(t) < len(cols) - 1:
            continue
        r = {}
        for c, v in zip(cols, t):
            try:
                r[c] = float(v)
            except ValueError:
                r[c] = float("nan")
        rows.append(r)
    return rows

def med(x):
    x = [v for v in x if v == v]
    return st.median(x) if x else float("nan")

def mean(x):
    x = [v for v in x if v == v]
    return st.mean(x) if x else float("nan")

def analyze(path, fps, label=""):
    rows = parse(path)
    idx = [i for i, r in enumerate(rows) if r.get("sm", 0) >= 50]
    if not idx:
        return {"label": label, "error": "no busy samples", "samples": len(rows)}
    a, b = idx[0] + 3, idx[-1] - 3
    w = rows[a:b + 1]
    g = lambda c: [r.get(c, float("nan")) for r in w]
    P, U = mean(g("pwr")), mean(g("sm"))
    rx, tx = mean(g("rxpci")), mean(g("txpci"))
    d = dict(label=label, window_s=len(w), fps=fps, power_w=round(P, 1), power_max_w=max(g("pwr")), sm_pct=round(U, 1), mem_pct=round(mean(g("mem")), 1),
             sm_mhz_med=med(g("pclk")), mem_mhz_med=med(g("mclk")), pcie_rx_mbs=round(rx, 1), pcie_tx_mbs=round(tx, 1),
             temp_c_med=med(g("gtemp")), fb_mb_med=med(g("fb")), pviol_pct_mean=round(mean(g("pviol")), 1), tviol_max=max([v for v in g("tviol") if v == v] or [float("nan")]))
    d.update(j_per_frame=round(P / fps, 4), gpu_ms_per_frame=round(U / 100 / fps * 1000, 3), frame_ms=round(1000 / fps, 3),
             pcie_mb_per_frame=round((rx + tx) / fps, 4), rx_mb_per_frame=round(rx / fps, 4), tx_mb_per_frame=round(tx / fps, 4), w_per_sm_pct=round(P / U, 3), fps_per_w=round(fps / P, 3))
    return d

if __name__ == "__main__":
    lab = sys.argv[sys.argv.index("--label") + 1] if "--label" in sys.argv else ""
    print(json.dumps(analyze(sys.argv[1], float(sys.argv[2]), lab)))
