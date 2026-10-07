#!/usr/bin/env python3
"""triage.py <resdir> [res-file] — one line per guest GSET_RES row with the evidence that names its cause:
item | verdict | host Xid (first) | guest Xid/NVRM (first) | kf3 RC line (first) | first non-baseline kf3 refusal.
Baseline noise dropped from the refusal column (measured on passing apps, V3_APP_MATRIX triage.py):
`QueueNotBound`, the AllocClassNotPermitted rows for class 50031 and NV40_I2C, the `kf3: family=` census."""
import os, re, sys
R = sys.argv[1]; RES = sys.argv[2] if len(sys.argv) > 2 else "guest.res"
NOISE = re.compile(r"QueueNotBound|class: 50031|NV40_I2C|kf3: family=")
def read(p):
    try: return open(p, errors="replace").read().splitlines()
    except OSError: return []
def clip(s, n): return re.sub(r"\s+", " ", s).strip().replace("|", "/")[:n]
base_dir = os.path.dirname(os.path.join(R, RES))
for line in read(os.path.join(R, RES)):
    if not line.startswith("GSET_RES "): continue
    kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", line[9:].strip()))
    it, v = kv.get("item"), kv.get("verdict")
    b = os.path.join(base_dir, it)
    hx = [l for l in read(b + ".host_dmesg.log") if "Xid" in l]
    gx = [l for l in read(b + ".guest_dmesg.log") if re.search(r"Xid|NVRM: .*(fail|error|timeout)", l, re.I)]
    kf = read(b + ".kf3.log")
    rc = [l for l in kf if re.search(r"RC host twin|RC_TRIGGERED|Xid", l)]
    ref = [l for l in kf if re.search(r"refus", l, re.I) and not NOISE.search(l)]
    uns = sorted(set(re.sub(r"seq(uence)?[=:] ?\d+", "", l.split("UNSERVICED", 1)[1]).strip()[:90] for l in kf if "GSP rpc UNSERVICED" in l))
    print(f"{it}|{v}|hXid:{clip(hx[0],150) if hx else '-'}|g:{clip(gx[0],120) if gx else '-'}|"
          f"kf3rc:{clip(rc[0],150) if rc else '-'}|kf3ref:({len(ref)}) {clip(ref[0],200) if ref else '-'}|"
          f"unserviced:{clip(';'.join(uns),200) if uns else '-'}|{clip(kv.get('note',''),120)}")
