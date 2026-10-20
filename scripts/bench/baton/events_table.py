#!/usr/bin/env python3
"""events_table.py RUNS_DIR [MIN_GAP_S=1.0] - one line per starvation event: session, run, starved client,
start/end/duration, the guest watchdog verdict (what the main thread was blocked in, cuStreamQuery) and the
session's busy_looks (last relay loop summary).  Tab separated."""
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import campaign_report as cr  # noqa: E402
import wd_parse as wp  # noqa: E402


def busy_looks(session_dir):
    last = None
    for p in glob.glob(os.path.join(session_dir, "fast_*_qemu.log")):
        with open(p, errors="replace") as f:
            for line in f:
                if "USER_RING_LOOP SUMMARY" in line:
                    m = re.search(r"busy_looks=(\d+)", line)
                    last = int(m.group(1)) if m else 0
    return "n/a" if last is None else str(last)


def main(argv):
    runs_dir = argv[0]
    min_gap = float(argv[1]) if len(argv) > 1 else 1.0
    print("session\trun\tstarved\tfrom_s\tto_s\tdur_s\tbusy_looks(session)\tguest_picture")
    for d in sorted(glob.glob(os.path.join(runs_dir, "*", "files"))):
        sess = os.path.dirname(d)
        bl = busy_looks(sess)
        for name in cr.group_runs(d):
            r = cr.analyze_run(d, name, min_gap)
            for ev in r["events"]:
                pic = "no watchdog episode (stall < 2 s)"
                for w in r["wd"]:
                    if not w.endswith(".%s.wd" % ev["starved"]) or os.path.getsize(w) == 0:
                        continue
                    with open(w, errors="replace") as f:
                        for e in wp.parse(f.read()):
                            if e["label"] == ev["starved"] and abs(e["t"] - ev["from_s"] - e["idle_ms"] / 1e3) < 1.5:
                                v = wp.classify(e)
                                pic = "%s; main: %s; streamq=%s" % (v["kind"], v["main_wait"], e["streamq_name"])
                print("%s\t%s\t%s\t%.1f\t%.1f\t%.2f\t%s\t%s" % (os.path.basename(sess), name, ev["starved"], ev["from_s"],
                                                           ev["to_s"], ev["gap_s"], bl, pic))


if __name__ == "__main__":
    main(sys.argv[1:])
