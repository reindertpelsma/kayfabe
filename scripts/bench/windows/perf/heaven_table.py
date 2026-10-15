#!/usr/bin/env python3
import glob, re, os, statistics as st, sys
rows = {}
for f in sorted(glob.glob("/data/claude-perf/ev/heaven/*.out")):
    n = os.path.basename(f)[:-4]
    t = open(f, errors="replace").read()
    t = re.sub(r"&nbsp;", " ", t)
    m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+).*?Min FPS:\s+([\d.]+).*?Max FPS:\s+([\d.]+).*?Score:\s+([\d.]+)", t.replace("\n", " "))
    mt = re.search(r"Time:\s+([\d.]+?)\d{2}:\d\d:\d\d Frames", t.replace("\n", " "))
    if m: rows[n] = (float(m.group(2)), float(m.group(3)), float(m.group(4)), float(m.group(5)), int(m.group(1)))
for n, v in rows.items(): print("%-18s fps %8.2f  min %7.2f  max %7.2f  score %8.1f  frames %d" % ((n,) + v))
def med(names):
    xs = [rows[n][0] for n in names if n in rows]
    return (st.median(xs), (max(xs) - min(xs)) / st.median(xs) * 100, len(xs)) if xs else None
for lab, names in [("K prod  D3D11", ["k_d3d11_r1", "k_d3d11_r2", "k_d3d11_r3"]), ("K prod  GL", ["k_gl_r1", "k_gl_r2", "k_gl_r3"]),
                   ("K scratch(persistent) D3D11", ["x_d3d11_r1", "x_d3d11_r2"]), ("VFIO D3D11", ["v_d3d11_r1", "v_d3d11_r2", "v_d3d11_r3"]), ("VFIO GL", ["v_gl_r1", "v_gl_r2", "v_gl_r3"])]:
    print(lab, med(names))
