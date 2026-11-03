"""Judge host NVRM Xid lines against ONE exact, declared exception.

Default: any Xid is FATAL. Exception (arm `off_c` and every `on_*` arm; owner, 2026-10-22): at most two
Xid 32 raised by the arm's own QEMU pid with intr 00200000, only after full init (Code 0 / init beyond the
display objects), in the observed shape: at most one plain channel id (< 0x100) and at most one `0x0e0000xx`
id, never the same channel twice. Anything else is FATAL; no Xid is CLEAN.
This is PRODUCT BUG B1 (Translated host rings are born without an error notifier, so kayfabe never sees the
RC), recorded as a bug, not as a baseline.
Usage: xid_policy.py ARM QEMU_PID CODE0(0|1) < kernel-journal-slice   (exit 0 CLEAN/EXPECTED, 1 FATAL)
"""
import json
import re
import sys

XID = re.compile(r"NVRM: Xid \(PCI:[^)]*\): (\d+), pid=(\d+), name=qemu-system-x86(?:_64)?, channel (0x[0-9a-fA-F]+) intr ([0-9a-fA-F]{8})\s*$")
EXCEPTION_ARMS = ("off_c", "on_")  # exact name, or prefix for the on_* arms
EXCEPTION_CODE = 32
EXCEPTION_INTR = "00200000"
EXCEPTION_MAX = 2


def judge(lines, arm: str, qemu_pid: int, code0_seen: bool) -> dict:
    xids = []
    unparsed = []
    for l in lines:
        m = XID.search(l) if l.count("NVRM: Xid") == 1 else None
        if m:
            xids.append({"code": int(m[1]), "pid": int(m[2]), "channel": m[3], "intr": m[4], "line": l.strip()[:240]})
        elif "NVRM: Xid" in l:
            unparsed.append(l.strip()[:240])
    if unparsed:
        return {"verdict": "FATAL", "xids": xids, "unparsed_xids": unparsed}
    if not xids:
        return {"verdict": "CLEAN", "xids": []}
    chans = [int(x["channel"], 16) for x in xids]
    plain = [c for c in chans if c < 0x100]
    high = [c for c in chans if c & ~0xFF == 0x0E000000]
    shape_ok = len(set(chans)) == len(chans) and len(plain) <= 1 and len(high) <= 1 and len(plain) + len(high) == len(chans)
    arm_ok = arm == "off_c" or arm.startswith("on_")
    allowed = (
        arm_ok
        and qemu_pid > 1
        and shape_ok
        and code0_seen
        and len(xids) <= EXCEPTION_MAX
        and all(x["code"] == EXCEPTION_CODE and x["pid"] == qemu_pid and x["intr"] == EXCEPTION_INTR for x in xids)
    )
    return {"verdict": "EXPECTED" if allowed else "FATAL", "xids": xids}


def main(argv):
    v = judge(sys.stdin.read().splitlines(), argv[1], int(argv[2]), argv[3] == "1")
    print(json.dumps(v))
    return 0 if v["verdict"] != "FATAL" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
