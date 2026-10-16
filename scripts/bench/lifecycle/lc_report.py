#!/usr/bin/env python3
"""lc_report.py QEMU_LOG HEAVEN_OUT FPS [TRIM_START TRIM_END] : table of the `kf3: LIFECYCLE` counters between two dumps
inside the benchmark window taken from Heaven's own log lines ("Benchmark running" .. "Benchmark results", guest UTC clock,
trimmed by TRIM_START / TRIM_END seconds; defaults 8 / 6). Output is text for the report."""
import re, sys, json, ast

import datetime
log, hout, fps = sys.argv[1], sys.argv[2], float(sys.argv[3])
T0 = float(sys.argv[4]) if len(sys.argv) > 4 else 8.0
T1 = float(sys.argv[5]) if len(sys.argv) > 5 else 6.0
txt = open(hout, errors="replace").read().replace("&nbsp;", " ")
def hms(m):
    h, mi, se = map(int, m.groups()); d = datetime.datetime.now(datetime.timezone.utc).replace(hour=h, minute=mi, second=se, microsecond=0)
    return d.timestamp()
mrun = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark running", txt)
mres = re.search(r"(\d\d):(\d\d):(\d\d) Benchmark results", txt)
bstart, bend = hms(mrun), hms(mres)
f9, W0, W1 = bstart, T0, (bend - bstart) - T1
kv = re.compile(r'(\w+)=(\[[^\]]*\]|\S+)')
dumps = {}  # wall -> {key: {field: value}}
for line in open(log, errors="replace"):
    if "kf3: LIFECYCLE " not in line:
        continue
    m = re.search(r'LIFECYCLE t=([\d.]+) wall=([\d.]+) (\w+) (.*)$', line)
    if not m:
        continue
    t, wall, why, rest = float(m.group(1)), float(m.group(2)), m.group(3), m.group(4)
    if rest.startswith("read "):
        f = dict(kv.findall(rest[5:]))
        key = "read:%s:%s" % (f.pop("site"), f.pop("class"))
    elif rest.startswith("pb_len "):
        f = dict(kv.findall(rest[7:])); key = "pb_len:%s" % f.pop("class")
    elif rest.startswith("gp "):
        f = dict(kv.findall(rest[3:])); key = "gp"
    elif rest.startswith("loop "):
        f = dict(kv.findall(rest[5:])); key = "loop"
        # registry=live=.. registered=... is a multi-field tail: keep raw
        f["registry"] = rest.split("registry=", 1)[-1]
    elif rest.startswith("relay_stats "):
        f = {"raw": rest[12:]}; key = "relay"
    else:
        continue
    dumps.setdefault(wall, {})[key] = f
walls = sorted(dumps)
if not walls:
    print("no LIFECYCLE dumps in", log); sys.exit(1)
def at(w):
    c = [x for x in walls if x <= w]
    return c[-1] if c else walls[0]
a, b = at(f9 + W0), at(f9 + W1)
dt = b - a
print("window: f9+%.0f..f9+%.0f -> dumps wall %.1f .. %.1f (%.1f s); %d dumps total; frames in window ~ %.0f at %.2f fps" % (W0, W1, a, b, dt, len(walls), fps * dt, fps))
A, B = dumps[a], dumps[b]
def d(key, field):
    try:
        return int(B[key][field]) - int(A[key][field])
    except Exception:
        return None
frames = fps * dt
print("\n== reads by site and lifecycle class (per second | per frame)")
print("%-28s %10s %10s %10s %10s %10s %12s %12s" % ("site:class", "calls/f", "KB/f", "ramKB/f", "vramKB/f", "vcopies/f", "vram_us/f", "total_us/f"))
for key in sorted(B):
    if not key.startswith("read:"):
        continue
    g = lambda f: (d(key, f) or 0) / frames
    print("%-28s %10.1f %10.1f %10.1f %10.1f %10.1f %12.1f %12.1f" % (key[5:], g("calls"), g("bytes") / 1024, g("ram_bytes") / 1024, g("vram_bytes") / 1024, g("vram_copies"), g("vram_ns") / 1000, g("total_ns") / 1000))
tot_v = sum((d(k, "vram_copies") or 0) for k in B if k.startswith("read:pushbuffer:"))
print("\n== HA: BAR1 pushbuffer copies by class (window total %d, %.1f per frame)" % (tot_v, tot_v / frames))
for c in ("pre5080", "post5080_nopend", "post5080_regpend"):
    k = "read:pushbuffer:" + c
    v = (d(k, "vram_copies") or 0) if k in B else 0
    print("  %-18s %10d  %5.1f%%" % (c, v, 100.0 * v / tot_v if tot_v else 0))
print("\n== pushbuffer length buckets (calls | vram copies), whole window")
for key in sorted(B):
    if key.startswith("pb_len:"):
        buckets = ast.literal_eval(B[key]["buckets"])
        ca, cb = ast.literal_eval(A.get(key, B[key])["calls"]), ast.literal_eval(B[key]["calls"])
        va, vb = ast.literal_eval(A.get(key, B[key])["vram_copies"]), ast.literal_eval(B[key]["vram_copies"])
        print(" ", key[7:], {bk: (cb[i] - ca[i], vb[i] - va[i]) for i, bk in enumerate(buckets)})
print("\n== GP entries (window total | per frame)")
for f in ("gp_entries", "len0", "sync_wait", "sync_wait_len0", "sync_wait_len_gt0"):
    v = d("gp", f)
    print("  %-20s %10s %10.2f" % (f, v, (v or 0) / frames))
ge, sw = d("gp", "gp_entries") or 0, d("gp", "sync_wait") or 0
print("  sync_wait share of GP entries: %.2f%%" % (100.0 * sw / ge if ge else 0))
print("\n== loop counters (window total | per frame | per second)")
for f in ("turns", "turns_published", "turns_retry", "rare_segments", "trig_update_channel_info", "trig_invalidate_tlb", "trig_context", "trig_other", "ring_migrated", "pb_reads_before_5080", "scanoff_skipped", "scanoff_skipped_bytes", "doorbells_translated", "doorbells_other"):
    v = d("loop", f)
    if v is not None:
        print("  %-26s %12d %10.2f %12.1f" % (f, v, v / frames, v / dt))
print("  registry (end):", B["loop"].get("registry", "")[:300])
print("  registry (start):", A["loop"].get("registry", "")[:300])
print("\n== relay_stats (end) :", B.get("relay", {}).get("raw", "")[:900])
print("== relay_stats (start):", A.get("relay", {}).get("raw", "")[:900])
def wk(s):
    return {m.group(1): int(m.group(2)) for m in re.finditer(r'(doorbell|event|spin|backstop)=(\d+)', s.split(")")[0])}
try:
    wa, wb = wk(A["relay"]["raw"]), wk(B["relay"]["raw"])
    print("\n== wakes in window:", {k: (wb[k] - wa[k], round((wb[k] - wa[k]) / frames, 2), "/frame") for k in wb})
except Exception as e:
    print("wakes parse failed", e)
