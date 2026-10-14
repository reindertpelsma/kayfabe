import re, sys, os
base = sys.argv[1]
runs = [("HW AD104 (bare metal, 580.159.04 open)", "hw-ad104-vast/all.log"),
        ("kf bf510bdf (before)", "run1-bf510bdf/all.log"),
        ("kf a34d3003", "run2-a34d3003/all.log"),
        ("kf 113a7535", "run3-113a7535/all.log"),
        ("kf 41acc6ac", "run4-41acc6ac/all.log")]
order = []
data = {}
for name, f in runs:
    p = os.path.join(base, f)
    if not os.path.exists(p):
        continue
    for line in open(p, errors="replace"):
        m = re.match(r"KFPLANES_(RESULT|DISABLE|CYCLE|DPMS) (.*)", line.strip())
        if not m:
            continue
        kind, rest = m.groups()
        kv = dict(re.findall(r'(\w+)=("[^"]*"|\S+)', rest))
        t = kv.get("test", kind)
        if kind == "DISABLE":
            key = "disable after " + t
            val = ("event %s %s" % (kv.get("event"), ("%.1f ms" % (float(kv["lat_us"]) / 1000)) if kv.get("lat_us", "-1") != "-1" else "")) if kv.get("rc") == "0" else "commit rc=%s (nothing to disable)" % kv.get("rc")
        elif kind == "CYCLE":
            key = t + " (disables)"
            val = "%s/%s disable events" % (kv.get("disable_events"), kv.get("cycles"))
        elif kind == "DPMS":
            continue
        else:
            key = t
            if kv.get("verdict") == "SKIP":
                val = "SKIP " + kv.get("why", "").strip('"')[:40]
            elif t in ("out-fence", "in-fence", "dpms-cycle"):
                if t == "dpms-cycle":
                    val = "off max %.1f ms, on max %.1f ms" % (float(kv["off_max_us"]) / 1000, float(kv["on_max_us"]) / 1000)
                elif t == "out-fence":
                    val = "%s/%s signalled, late %s, max %.1f ms" % (kv.get("signalled"), kv.get("commits"), kv.get("late1f"), float(kv.get("lat_max_us", 0)) / 1000)
                else:
                    val = "%s/%s ev, early %s, after-signal max %.1f ms" % (kv.get("events"), kv.get("commits"), kv.get("early"), float(kv.get("after_signal_max_us", 0)) / 1000)
            elif "events" in kv:
                val = "%s/%s ev, %s tmo, p95 %.1f ms, max %.1f ms" % (
                    kv.get("events"), kv.get("commits"), kv.get("timeouts"),
                    float(kv.get("lat_p95_us", 0)) / 1000, float(kv.get("lat_max_us", 0)) / 1000)
            elif t == "dpms-cycle":
                val = "off max %.1f ms, on max %.1f ms" % (float(kv["off_max_us"]) / 1000, float(kv["on_max_us"]) / 1000)
            elif t == "out-fence":
                val = "%s/%s signalled, late %s, max %.1f ms" % (kv.get("signalled"), kv.get("commits"), kv.get("late1f"), float(kv.get("lat_max_us", 0)) / 1000)
            elif t == "in-fence":
                val = "%s/%s ev, early %s, after-signal max %.1f ms" % (kv.get("events"), kv.get("commits"), kv.get("early"), float(kv.get("after_signal_max_us", 0)) / 1000)
            else:
                val = kv.get("verdict", "?") + " " + (("err " + kv.get("last_err", "")) if kv.get("errors", "0") != "0" else "")
        if key not in order:
            order.append(key)
        data.setdefault(key, {})[name] = val
cols = [n for n, f in runs if os.path.exists(os.path.join(base, f))]
print("| test | " + " | ".join(cols) + " |")
print("|---|" + "---|" * len(cols))
for k in order:
    print("| " + k + " | " + " | ".join(data[k].get(c, "-") for c in cols) + " |")
