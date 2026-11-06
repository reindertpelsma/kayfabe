#!/usr/bin/env python3
"""summarize.py DIR... : one block per (case, rep) log: the RES/RM-status lines, the pushbuffer words, and the host
journal lines that carry the probe's pid. Usage: python3 -I summarize.py out/a_1 out/a_2 ..."""
import os
import re
import sys


def main(dirs):
    for d in dirs:
        names = sorted(f for f in os.listdir(d) if f.endswith(".log"))
        for n in names:
            base = n[:-4]
            log = open(os.path.join(d, n), errors="replace").read().splitlines()
            meta = ""
            mp = os.path.join(d, base + ".meta")
            if os.path.exists(mp):
                meta = open(mp).read().strip()
            m = re.search(r"pid=(\d+)", meta)
            pid = m.group(1) if m else "?"
            jp = os.path.join(d, base + ".journal")
            jl = open(jp, errors="replace").read().splitlines() if os.path.exists(jp) else []
            mine = [x for x in jl if f"pid={pid}," in x]
            print("=" * 100)
            print(f"{base}  {meta}")
            for ln in log:
                if ln.startswith(("RES", "OBS")) or "status=0x" in ln and ("alloc" in ln or "control" in ln or "free" in ln):
                    print("  " + ln)
            print(f"  JOURNAL lines naming pid {pid}: {len(mine)}")
            for x in mine:
                print("    " + x)
            other = [x for x in jl if f"pid={pid}," not in x]
            if other:
                print(f"  (journal window also holds {len(other)} NVRM lines of other pids, not attributed)")


if __name__ == "__main__":
    main(sys.argv[1:])
