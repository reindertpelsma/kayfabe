#!/usr/bin/env python3
import json, re, sys, pathlib
E = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path("ev")
rows = []
for tag, plat in (("s1v", "vfio"), ("s2k", "kf3")):
    for n in (1, 2, 3):
        d = E / ("%s_a%d" % (tag, n))
        if not d.exists():
            continue
        h = d / "h"
        s = json.loads((d / "summary.json").read_text())
        res = (h / "result.md").read_text(errors="replace")
        m = re.search(r"RTC base (\S+)", res)
        base = m.group(1) if m else "1970"
        soak = (h / "soak.txt").read_text(errors="replace").splitlines() if (h / "soak.txt").exists() else []
        alive = [l for l in soak if "ALIVE" in l]
        dead = [l for l in soak if "ALIVE" not in l and l.startswith("t+")]
        last = re.search(r"ALIVE (\d+)", alive[-1]).group(1) if alive else "-"
        ev = ""
        for f in sorted(h.glob("app_log_events_*.txt")):
            ev += f.read_text(errors="replace") + "\n"
        new = [l for l in ev.splitlines() if re.match(r"\d{4}-\d\d-\d\dT", l) and l[:19] > base[:19]]
        crash = [l for l in new if "id=1000" in l]
        lke = [l for l in new if "LiveKernelEvent" in l or "nvlddmkm" in l or "id=153" in l]
        resolve_lines = [l for l in new if re.search(r"Resolve|nvoglv64", l)]
        dm = (d / "dmesg_new_filtered.txt").read_text(errors="replace") if (d / "dmesg_new_filtered.txt").exists() else ""
        xid = len(re.findall(r"Xid", dm))
        cen = s.get("census") or {}
        rows.append(dict(attempt="%s_a%d" % (tag, n), platform=plat, dur=s["duration_s"], alive_s=last, dead_polls=len(dead), crash_1000=len(crash),
                         resolve_or_nvoglv64=len(resolve_lines), lke_nvlddmkm=len(lke), xid=xid, unserviced=cen.get("unserviced", "n/a"), refused=cen.get("refused", "n/a"),
                         row=s["row_lines"][0][:90] if s["row_lines"] else ""))
print("| attempt | platform | duration s | Resolve alive at last soak poll s | dead polls | Application Error 1000 after clock base | Resolve/nvoglv64 events | LKE/nvlddmkm | host Xid | kf3 unserviced | kf3 refused |")
print("|---|---|---|---|---|---|---|---|---|---|---|")
for r in rows:
    print("| %(attempt)s | %(platform)s | %(dur)s | %(alive_s)s | %(dead_polls)s | %(crash_1000)s | %(resolve_or_nvoglv64)s | %(lke_nvlddmkm)s | %(xid)s | %(unserviced)s | %(refused)s |" % r)
print()
for r in rows:
    print(r["attempt"], r["row"])
