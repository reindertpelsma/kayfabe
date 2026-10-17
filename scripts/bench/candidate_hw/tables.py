#!/usr/bin/env python3
"""tables.py <windows-evidence-dir> : the per-boot fps table and the lazy-observe counter table from the collected text evidence (collect_win.sh output)."""
import re, sys, os
D = sys.argv[1]
MAIN = "P1 P2 BO1 BO2 AO1 AO2 OFF1 OFF2".split(); CTL = "C1 C2 CM2".split()
def kv(s): return {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", s)}
def hv(t, api):
    s = open(f"{D}/{t}/heaven_{api}.txt", errors="replace").read()
    m = re.search(r"Time:\s+([\d.]+)\s+Frames:\s+(\d+)\s+FPS:\s+([\d.]+)\s+Min FPS:\s+([\d.]+)\s+Max FPS:\s+([\d.]+)\s+Score:\s+([\d.]+)", s)
    ev = open(f"{D}/{t}/events_{api}.txt").read(); g = re.search(r"gpu_driver=(\d+)", ev)
    d = open(f"{D}/{t}/dmon_summary.txt").read(); p = re.search(rf"dmon_{api} rows=\d+ avg_power_w=([\d.]+) avg_sm_pct=([\d.]+)", d)
    return float(m[3]), float(m[4]), float(m[5]), int(m[2]), float(m[6]), int(g[1]), p[1], p[2]
print("| boot | D3D11 fps | min | max | frames | score | GPU W | SM% | GL fps | min | max | GL W | SM% | nvlddmkm events in the D3D11 / GL 9-minute window |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
for t in MAIN + CTL:
    a = hv(t, "d3d11"); b = hv(t, "gl")
    print(f"| {t} | {a[0]:.1f} | {a[1]:.1f} | {a[2]:.0f} | {a[3]} | {a[4]:.0f} | {a[6]} | {a[7]} | {b[0]:.1f} | {b[1]:.1f} | {b[2]:.0f} | {b[6]} | {b[7]} | {a[5]} / {b[5]} |")
print()
print("| boot | API | relay | turns | identity | hits/turns | skipped/turns | low_credit/turns | interval/(turns/16) | forced_wait | ring_full | inval mode/migr/reset/retire | pointer_markers | pointer_backstop | section-11 zero |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
for t in MAIN:
    L = open(f"{D}/{t}/user_ring_lines.txt", errors="replace").read(); lazy = {}; life = {}
    for m in re.finditer(r"USER_RING_LAZY_OBSERVE \((\w+), (\w+)\) (turns=.*)", L): lazy[(m[1], m[2])] = kv(m[3])
    for m in re.finditer(r"USER_RING_LIFECYCLE \((\w+), (\w+)\) mode=\w+ Counts \{(.*?)\}", L): life[(m[1], m[2])] = kv(m[3].replace(": ", "="))
    big = sorted(sorted(lazy.items(), key=lambda x: -x[1]["turns"])[:2], key=lambda x: int(x[0][0], 16))
    for (k, z), api in zip(big, ("D3D11", "GL")):
        l = life[k]; T = z["turns"]
        ident = z["obs_skipped"] + z["obs_unconditional"] + sum(z[a] for a in z if a.startswith("obs_forced_")) == T
        zero = all(l[a] == 0 for a in "put_boundary_open_scan put_boundary_open_window window_fresh_start_parse_error switch_cursor_mismatch pointer_pb_read_violation window_binding_changed anchor_stale_start pointer_markers_latched_off after_teardown".split())
        print(f"| {t} | {api} | {k[0][-3:]}:{k[1][-2:]} | {T/1e6:.1f} M | {'yes' if ident else 'NO'} | {z['put_cache_hits']/T:.4f} | {z['obs_skipped']/T:.3f} | {z['obs_forced_low_credit']/T:.3f} | {z['obs_forced_interval']/(T/16):.2f} | {z['obs_forced_wait']} | {z['obs_forced_ring_full']} | {z['inval_mode_change']}/{z['inval_migration']}/{z['inval_reset']}/{z['inval_retire']} | {l['pointer_markers']} | {l['pointer_backstop']} | {'yes' if zero else 'NO'} |")
