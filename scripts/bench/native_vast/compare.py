#!/usr/bin/env python3
"""compare.py KAYFABE_ROWS.jsonl NATIVE_SUMMARY.json [NATIVE_SUMMARY2.json ...] : the native-vs-kayfabe table.
Kayfabe scan-off = m2exp1-3 (D3D11 and OpenGL, origin/claude/perf-lifecycle-measure-20261016); PCIe/frame comes from the host dmon files that carry rx/tx
(pfm, pfm2 = scan-off D3D11; pfc/pn2/pr1 GL = scan-on OpenGL, the only complete GL runs with PCIe columns)."""
import json, statistics as st, sys
rows = [json.loads(l) for l in open(sys.argv[1])]
def sel(names):
    return [r for r in rows if r["label"] in names]
def med(rs, k):
    x = [r[k] for r in rs if r.get(k) == r.get(k)]
    return st.median(x) if x else float("nan")
K = ["fps", "power_w", "sm_pct", "j_per_frame", "gpu_ms_per_frame", "frame_ms", "w_per_sm_pct", "fps_per_w"]
kd = sel(["m2exp1_d3d11", "m2exp2_d3d11", "m2exp3_d3d11"]); kg = sel(["m2exp1_gl", "m2exp2_gl", "m2exp3_gl"])
kd_p = sel(["pfm_d3d11", "pfm2_d3d11"]); kg_p = sel(["pfc_gl", "pn2_gl", "pr1_gl"])
kay = {"d3d11": {k: med(kd, k) for k in K}, "gl": {k: med(kg, k) for k in K}}
kay["d3d11"]["pcie_mb_per_frame"] = med(kd_p, "pcie_mb_per_frame"); kay["gl"]["pcie_mb_per_frame"] = med(kg_p, "pcie_mb_per_frame")
def ratios(d, g):
    return {"r_fps": d["fps"] / g["fps"], "r_E": d["j_per_frame"] / g["j_per_frame"], "r_G": d["gpu_ms_per_frame"] / g["gpu_ms_per_frame"], "r_P": d["pcie_mb_per_frame"] / g["pcie_mb_per_frame"], "r_W": d["w_per_sm_pct"] / g["w_per_sm_pct"]}
cols = [("kayfabe scan-off", kay)] + [(p.split("/")[-1].replace("_summary.json", ""), json.load(open(p))) for p in sys.argv[2:]]
F = ["fps", "power_w", "sm_pct", "j_per_frame", "gpu_ms_per_frame", "frame_ms", "pcie_mb_per_frame", "w_per_sm_pct", "fps_per_w"]
print("%-20s" % "metric (median)" + "".join("%-30s" % (n + " " + a) for n, _ in cols for a in ("D3D11", "GL")))
for k in F:
    print("%-20s" % k + "".join("%-15.3f%-15.3f" % (c["d3d11"].get(k, float("nan")), c["gl"].get(k, float("nan"))) for _, c in cols))
print()
for n, c in cols:
    r = ratios(c["d3d11"], c["gl"]); print("RATIOS D3D11/GL %-18s" % n, " ".join("%s=%.3f" % (k, v) for k, v in r.items()))
