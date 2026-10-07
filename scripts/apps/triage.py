#!/usr/bin/env python3
"""triage.py <results-dir> [res-file] — one line per guest APPRES row with the evidence that names
its cause: app | verdict | guest Xid lines | kf3 RC/Xid lines | first non-baseline kf3 refusal | note.

Baseline noise (present on every passing app, measured 670bd310 on vh) is dropped from the refusal
column: `QueueNotBound`, the AllocClassNotPermitted rows for class 50031 (0xc36f... allowlist) and
NV40_I2C, and the `kf3: family=` census line.  Everything else is shown, first occurrence only."""
import os, re, sys

R = sys.argv[1]
RES = sys.argv[2] if len(sys.argv) > 2 else "guest.res"
NOISE = re.compile(r"QueueNotBound|class: 50031|NV40_I2C|kf3: family=")

def read(p):
    try:
        return open(p, errors="replace").read().splitlines()
    except OSError:
        return []

def clip(s, n):
    s = re.sub(r"\s+", " ", s).strip().replace("|", "/")
    return s[:n]

for line in read(os.path.join(R, RES)):
    if not line.startswith("APPRES "):
        continue
    kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", line[7:].strip()))
    app, v = kv.get("app"), kv.get("verdict")
    base = os.path.join(R, app)
    gx = [l for l in read(base + ".guest_dmesg.log") if "Xid" in l]
    kf = read(base + ".kf3.log")
    rc = [l for l in kf if re.search(r"RC host twin|Xid|RC_TRIGGERED", l)]
    ref = [l for l in kf if re.search(r"refus", l, re.I) and not NOISE.search(l)]
    gxs = clip(re.sub(r"^\[[^\]]*\] ", "", gx[0]), 150) if gx else "-"
    rcs = clip(rc[0], 170) if rc else "-"
    refs = f"({len(ref)}) " + clip(ref[0], 200) if ref else "-"
    print(f"{app}|{v}|{kv.get('rc')}|{kv.get('secs')}s|gXid:{gxs}|kf3rc:{rcs}|kf3ref:{refs}|{clip(kv.get('note',''),140)}")
