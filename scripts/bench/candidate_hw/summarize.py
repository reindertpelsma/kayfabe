#!/usr/bin/env python3
"""summarize.py <gapdir> TAG... : one block per Windows boot (tag): Heaven results of both APIs, sanity, environment, lazy / lifecycle counters of the two big relays.
Reads <gapdir>/ev/{heaven,arm}/. The D3D11 relay is the lower client handle of the two largest relays, the OpenGL relay the higher (INFERRED from allocation order)."""
import re, sys, os, json
G = sys.argv[1]; tags = sys.argv[2:]
KEYS11 = ["put_boundary_open_scan", "put_boundary_open_window", "window_fresh_start_parse_error", "switch_cursor_mismatch", "pointer_pb_read_violation",
          "window_binding_changed", "anchor_stale_start", "pointer_markers_latched_off", "after_teardown"]
def rd(p):
    try: return open(p, errors="replace").read()
    except OSError: return ""
def kv(s): return {k: int(v) for k, v in re.findall(r"(\w+)[=:] ?(\d+)", s)}
def heaven(tag, api):
    t = rd(f"{G}/ev/heaven/{tag.lower()}_{api}.out") + rd(f"{G}/ev/heaven/{tag.lower()}_{api}.sum")
    m = re.search(r"Time:\s+([\d.]+)\s+Frames:\s+(\d+)\s+FPS:\s+([\d.]+)\s+Min FPS:\s+([\d.]+)\s+Max FPS:\s+([\d.]+)\s+Score:\s+([\d.]+)", t)
    ev = re.search(r"events: gpu_driver=(\d+) other=(\d+)", t)
    d = {}
    if m: d = dict(time=float(m[1]), frames=int(m[2]), fps=float(m[3]), minfps=float(m[4]), maxfps=float(m[5]), score=float(m[6]))
    if ev: d.update(gpu_events=int(ev[1]), boot_events=int(ev[2]))
    dm = rd(f"{G}/ev/heaven/{tag.lower()}_{api}/host_dmon.txt"); pw = []; sm = []
    for l in dm.splitlines():
        f = l.split()
        if len(f) > 6 and re.match(r"\d\d:\d\d:\d\d", f[0]):
            try:
                if float(f[5]) > 50: pw.append(float(f[2])); sm.append(float(f[5]))
            except ValueError: pass
    if pw: d.update(power_w=round(sum(pw) / len(pw), 1), sm_pct=round(sum(sm) / len(sm), 1), dmon_s=len(pw))
    return d
def relays(tag):
    t = rd(f"{G}/ev/arm/{tag}.lines.txt"); out = {}
    for m in re.finditer(r"USER_RING_LAZY_OBSERVE \((\w+), (\w+)\) (turns=.*)", t):
        out.setdefault((m[1], m[2]), {}).update(lazy=kv(m[3]))
    for m in re.finditer(r"USER_RING_LIFECYCLE \((\w+), (\w+)\) mode=(\w+) Counts \{(.*?)\}", t):
        out.setdefault((m[1], m[2]), {}).update(life=kv(m[4]), mode=m[3])
    return t, out
for tag in tags:
    print(f"=== {tag}")
    env = rd(f"{G}/ev/arm/{tag}.environ.txt")
    print("env:", re.findall(r"exe=(\S+)", env)[:1], "sha256=" + (re.findall(r"sha256=(\w{16})", env) or ["?"])[0] + "...", [l for l in env.splitlines() if re.match(r"KF3_(PUT_CACHE|LAZY_OBSERVE|LIFECYCLE|SCAN|PRIV)", l)] or "no PUT_CACHE/LAZY_OBSERVE/LIFECYCLE/SCAN variable set", [l for l in env.splitlines() if l.startswith("WIN_")])
    for api in ("d3d11", "gl"): print(api, json.dumps(heaven(tag, api)))
    print("xid:", rd(f"{G}/ev/arm/{tag}.xid_summary.txt").strip())
    shot = rd(f"{G}/ev/heaven/shot_{tag}_d3d11.out") + rd(f"{G}/ev/guest/shot_{tag}_d3d11.log")
    print("shot:", re.findall(r"SHOT screen=\S+ mean_luma=\S+ std_luma=\S+ distinct_4bit_colours=\d+", shot)[:1])
    t, rl = relays(tag)
    print("mode lines:", [l[:160] for l in t.splitlines() if "LAZY_OBSERVE_MODE" in l or "LIFECYCLE_MODE" in l])
    print("REFUSED lines:", len(re.findall(r"USER_RING_REFUSED", t)), "| LATCHED_OFF:", len(re.findall(r"POINTER_MARKERS_LATCHED_OFF", t)), "| PUT_BOUNDARY_OPEN:", len(re.findall(r"USER_RING_PUT_BOUNDARY_OPEN", t)), "| MIGRATED:", len(re.findall(r"USER_RING_MIGRATED", t)), "| WINDOW_FRESH:", len(re.findall(r"USER_RING_WINDOW_FRESH", t)))
    big = sorted(((v.get("lazy", {}).get("turns", 0), k) for k, v in rl.items()), reverse=True)[:2]
    big = sorted(big, key=lambda x: int(x[1][0], 16))
    for (turns, k), api in zip(big, ("d3d11", "gl")):
        v = rl[k]; z = v.get("lazy", {}); L = v.get("life", {})
        idsum = z["obs_skipped"] + z["obs_unconditional"] + z["obs_forced_wait"] + z["obs_forced_low_credit"] + z["obs_forced_open_segment"] + z["obs_forced_ring_full"] + z["obs_forced_interval"]
        print(f"big relay {api} {k[0]}:{k[1]} mode={v.get('mode')} turns={z['turns']}")
        print("  lazy:", " ".join(f"{a}={b}" for a, b in z.items() if a != "turns"))
        print(f"  identity turns == skipped+unconditional+forced_*: {idsum == z['turns']} ({idsum} vs {z['turns']}); hits+fresh <= turns: {z['put_cache_hits'] + z['put_fresh_reads'] <= z['turns']}")
        print(f"  put_cache_hits/turns={z['put_cache_hits'] / z['turns']:.5f}  obs_skipped/turns={z['obs_skipped'] / z['turns']:.4f}  forced_low_credit/turns={z['obs_forced_low_credit'] / z['turns']:.4f}  forced_interval/(turns/16)={z['obs_forced_interval'] / (z['turns'] / 16):.3f}  forced_wait/pointer_markers={z['obs_forced_wait'] / max(1, L.get('pointer_markers', 1)):.1f}")
        print("  section-11 must-be-0:", {a: L.get(a) for a in KEYS11}, "ALL_ZERO" if all(L.get(a) == 0 for a in KEYS11) else "NONZERO")
        print("  lifecycle:", {a: L.get(a) for a in ("entries_scan", "entries_pointer", "entries_window", "to_window_fresh", "restarts", "pointer_markers", "pointer_carried_no_marker", "pointer_backstop", "pointer_backstop_loud", "anchor_refused_ring_full", "post_switch_scanned", "set_object_5080_total", "set_object_5080_rebinds")})
    small = [(k, v["lazy"]) for k, v in rl.items() if "lazy" in v and k not in [b[1] for b in big]]
    bad = [k for k, z in small if z["obs_skipped"] + z["obs_unconditional"] + z["obs_forced_wait"] + z["obs_forced_low_credit"] + z["obs_forced_open_segment"] + z["obs_forced_ring_full"] + z["obs_forced_interval"] != z["turns"] or z["inval_reset"] != 0]
    print(f"other relays: {len(small)} lines, identity/inval_reset violations: {bad or 'none'}; sum of forced_ring_full over all: {sum(v['lazy']['obs_forced_ring_full'] for v in rl.values() if 'lazy' in v)}")
