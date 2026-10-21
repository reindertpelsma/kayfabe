"""Judge host NVRM Xid lines against ONE exact, declared exception.

Default: any Xid is FATAL. Exception (arm `off_c` only): at most two Xid 32 raised by the arm's own QEMU pid
with intr 00200000, and only after the guest reached Code 0. Anything else is FATAL; no Xid is CLEAN.
Usage: xid_policy.py ARM QEMU_PID CODE0(0|1) < kernel-journal-slice   (exit 0 CLEAN/EXPECTED, 1 FATAL)
"""
import json
import re
import sys

XID = re.compile(r"NVRM: Xid \(PCI:[^)]*\): (\d+), pid=(\d+), name=\S+, channel (0x[0-9a-fA-F]+) intr ([0-9a-fA-F]+)")
EXCEPTION_ARM = "off_c"
EXCEPTION_CODE = 32
EXCEPTION_INTR = "00200000"
EXCEPTION_MAX = 2


def judge(lines, arm: str, qemu_pid: int, code0_seen: bool) -> dict:
    xids = []
    for l in lines:
        m = XID.search(l)
        if m:
            xids.append({"code": int(m[1]), "pid": int(m[2]), "channel": m[3], "intr": m[4], "line": l.strip()[:240]})
    if not xids:
        return {"verdict": "CLEAN", "xids": []}
    allowed = (
        arm == EXCEPTION_ARM
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
