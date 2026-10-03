#!/usr/bin/env python3
"""triage.py <results-dir> [res-file] — one line per guest APPRES row with the evidence that names
its cause: app | verdict | guest Xid lines | kf3 RC/Xid lines | first non-baseline kf3 refusal | note.

★ 2026-10-03 (release §I): a managed-memory row also carries its `loud_verdict.sh` class
(EXPECTED_LOUD / KF3_DEFECT / SILENT) after the verdict, and a closing `LOUD …` line counts them;
SILENT and KF3_DEFECT are release blockers. Rows recorded before the hook wrote `loud=` print as
before.

⊘ CORRECTED 2026-10-03 (review of 5af7e644): the kf3rc column matched the bare words
`RC host twin|Xid|RC_TRIGGERED`, and kf3's boot-report sentence (DELIVERY_UNBUILT, printed once per
QEMU at the first fault-buffer registration) names `Xid 31` and `RC_TRIGGERED` — so in an isolated
boot that sentence was the column's FIRST line, ahead of the real RC line. The column now matches
the shapes of the RC lines kf3 prints (`kf3: RC host twin 0x…`, `kf3: RC_TRIGGERED posted:`,
`kf3: OS_ERROR_LOG posted:`, `kf3: UNSERVICED-GPU-FAULT guest client`). The closing lines also name
every boot whose silent-twin gate (`APPS_BOOT_GATE`, boot_gate.sh) is not PASS.

⊘ CORRECTED 2026-10-03 (review of 9390f51c): those closing lines counted EVERY gate line, while
summarize.py kept the last per boot — and guest.res is appended to, so a reused run name made the two
disagree. They now come from boot_gates.py, the one rule apps_matrix.sh and summarize.py also use
(`BOOT_GATE boots= pass= fail= unmeasured= lane=`, over this res file).

Baseline noise (present on every passing app, measured 670bd310 on vh) is dropped from the refusal
column: `QueueNotBound`, the AllocClassNotPermitted rows for class 50031 (0xc36f... allowlist) and
NV40_I2C, and the `kf3: family=` census line.  Everything else is shown, first occurrence only."""
import collections, os, re, sys
import boot_gates  # the ONE rule for APPS_BOOT_GATE lines (beside this script; sys.path[0])

R = sys.argv[1]
RES = sys.argv[2] if len(sys.argv) > 2 else "guest.res"
NOISE = re.compile(r"QueueNotBound|class: 50031|NV40_I2C|kf3: family=")
RC_LINE = re.compile(r"kf3: (RC host twin 0x[0-9a-f]+ |RC_TRIGGERED posted: |OS_ERROR_LOG posted: "
                     r"|UNSERVICED-GPU-FAULT guest client 0x)")

def read(p):
    try:
        return open(p, errors="replace").read().splitlines()
    except OSError:
        return []

def clip(s, n):
    s = re.sub(r"\s+", " ", s).strip().replace("|", "/")
    return s[:n]

loud = collections.Counter()
rows = 0
for line in read(os.path.join(R, RES)):
    if not line.startswith("APPRES "):
        continue
    rows += 1
    kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", line[7:].strip()))
    app, v = kv.get("app"), kv.get("verdict")
    base = os.path.join(R, app)
    gx = [l for l in read(base + ".guest_dmesg.log") if "Xid" in l]
    kf = read(base + ".kf3.log")
    rc = [l for l in kf if RC_LINE.search(l)]
    ref = [l for l in kf if re.search(r"refus", l, re.I) and not NOISE.search(l)]
    gxs = clip(re.sub(r"^\[[^\]]*\] ", "", gx[0]), 150) if gx else "-"
    rcs = clip(rc[0], 170) if rc else "-"
    refs = f"({len(ref)}) " + clip(ref[0], 200) if ref else "-"
    lc = kv.get("loud", "-")
    if lc not in ("-", ""):
        loud[lc] += 1
        v = f"{v}/{lc}"
    print(f"{app}|{v}|{kv.get('rc')}|{kv.get('secs')}s|gXid:{gxs}|kf3rc:{rcs}|kf3ref:{refs}|{clip(kv.get('note',''),140)}")
if loud:
    blockers = loud["SILENT"] + loud["KF3_DEFECT"]
    print("LOUD " + " ".join(f"{k}={n}" for k, n in sorted(loud.items())) + f" blockers={blockers}")
if rows:
    head, bad, _lane = boot_gates.verdict([os.path.join(R, RES)])
    print(head)
    for b in bad:
        print(clip(b, 300))
