"""Narrow HM1 intentional CE-fault admission; coordinator ruling 2026-10-25.

Candidate 6564dd597, kf3-bins/6564dd59/qemu-system-x86_64.
Only predeclared inducing arms, own PID, exact CE/PTE/access shape and named VAs pass.
"""
import re

MARKERS = {"defer-liveness": "defer_liveness_prime",
           "missing-page-fault": "victim_release"}
XID = re.compile(
    r"NVRM: Xid \(PCI:[^)]+\): (?P<xid>\d+), pid=(?P<pid>\d+), "
    r".*?channel (?P<channel>0x[0-9a-fA-F]+),.*?MMU Fault: ENGINE "
    r"(?P<engine>\w+) (?P<client>\w+) faulted @ (?P<va>0x[0-9a-fA-F_]+)\. "
    r"Fault is of type (?P<kind>\w+) ACCESS_TYPE_(?P<access>\w+)"
)
MARK = re.compile(r"FAULT_MARK=(\w+)\b[^\r\n]*?\bva=(0x[0-9a-fA-F_]+)")
OOM = ("Out of memory:", "oom-kill:", "Killed process ")


def audit(lines, *, arm, pid, allowed_vas, prime_limit, serial):
    """Return rejected records; duplicate records at one primed VA count once."""
    rejected = []
    seen = set()
    declared = {int(va.replace("_", ""), 16) for name, va in MARK.findall(serial)
                if name == MARKERS.get(arm)}
    for line in lines:
        if any(word in line for word in OOM):
            rejected.append("oom: " + line)
            continue
        if "Xid" not in line:
            continue
        match = XID.search(line)
        if match is None:
            rejected.append("unparsed-xid: " + line)
            continue
        record = match.groupdict()
        va = int(record["va"].replace("_", ""), 16)
        seen.add(va)
        checks = (
            ("arm", arm in MARKERS),
            ("pid", int(record["pid"]) == pid),
            ("class", int(record["xid"]) == 31),
            ("engine", record["engine"] == "CE0"),
            ("client", record["client"] == "HUBCLIENT_CE1"),
            ("kind", record["kind"] == "FAULT_PTE"),
            ("access", record["access"] in ("VIRT_READ", "VIRT_WRITE")),
            ("va", va in allowed_vas),
            ("marker", va in declared),
            ("count", len(seen) <= prime_limit),
        )
        for name, valid in checks:
            if not valid:
                rejected.append(name + ": " + line)
    return rejected


def exit_code(reason, process_returncode):
    """Only an actual budget stop uses timeout status 124."""
    if reason == "budget":
        return 124
    return 3 if reason else process_returncode
