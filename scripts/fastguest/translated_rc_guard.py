#!/usr/bin/env python3
"""Exact intentional-control evidence gate; synthetic tests are not GPU evidence."""
import argparse
import json
from pathlib import Path
import re
import sys

MAX_BYTES = 2 * 1024 * 1024
RUN = r"[A-Za-z0-9_-]{1,32}"
DETAIL = r"intr1? [0-9a-f]{8}(?: HCE_DBG0 [0-9a-f]{8} HCE_DBG1 [0-9a-f]{8})?"


class Refused(ValueError):
    """The capture does not establish the requested error delivery."""


def check(manifest, qemu, guest, journal):
    """Check independent host and guest capture sources."""
    for data in (qemu, guest, journal):
        if not isinstance(data, str) or len(data.encode()) > MAX_BYTES:
            raise Refused("capture exceeds byte bound")
    if any(s in guest for s in ("Kernel panic", "RmInitAdapter failed", "Oops:", "GPU has fallen off")):
        raise Refused("guest-wide failure signature")
    try:
        run, mode, source = manifest["run"], manifest["mode"], manifest["source"]
        pid, pci = manifest["pid"], manifest["pci"]
        start, end, detail = manifest["start_ns"], manifest["end_ns"], manifest["detail"]
        valid = (re.fullmatch(RUN, run) and mode in ("on", "off", "native")
                 and re.fullmatch(r"[0-9a-f]{40}", source)
                 and type(pid) is int and 1 < pid < 2**31
                 and re.fullmatch(r"[0-9a-f]{4}:[0-9a-f]{2}:[0-9a-f]{2}", pci)
                 and type(start) is int and type(end) is int
                 and 0 < start < end and end-start <= 180_000_000_000
                 and ((mode == "native" and detail is None)
                      or (mode != "native" and re.fullmatch(DETAIL, detail))))
    except (KeyError, TypeError):
        valid = False
    if not valid:
        raise Refused("invalid predeclared manifest")

    def records(text, tag):
        # A malformed tagged record must not disappear through a selective regex.
        result = []
        for line in text.splitlines():
            if tag not in line:
                continue
            body = line[line.index(tag)+len(tag):].strip()
            row = {}
            for field in body.split():
                if "=" not in field:
                    raise Refused(f"malformed {tag} field")
                key, value = field.split("=", 1)
                if key in row or not value:
                    raise Refused(f"duplicate/empty {tag} field")
                row[key] = value
            result.append(row)
        return result

    def one(rows, why):
        if len(rows) != 1:
            raise Refused(f"expected one {why}, got {len(rows)}")
        return rows[0]

    def require(row, **fields):
        if any(row.get(k) != str(v) for k,v in fields.items()):
            raise Refused("capture identity/value mismatch")

    def number(row, key):
        try:
            text = row[key]
            if re.fullmatch(r"(?:0x[0-9a-f]+|[0-9]+)", text) is None:
                raise ValueError()
            return int(text, 0) if text.startswith("0x") else int(text)
        except (KeyError, ValueError):
            raise Refused(f"invalid numeric field {key}") from None

    xids = []
    try:
        for line in journal.splitlines():
            row = json.loads(line)
            message = row["MESSAGE"]
            timestamp = int(row["__REALTIME_TIMESTAMP"])*1000
            if not isinstance(message, str):
                raise ValueError()
            if "NVRM: Xid" in message:
                if not start <= timestamp <= end:
                    raise Refused("Xid outside capture interval")
                xids.append((timestamp, message))
    except (ValueError, KeyError, TypeError):
        raise Refused("malformed kernel journal") from None
    if mode == "native":
        if guest:
            raise Refused("native characterization cannot contain guest evidence")
        mark = one(records(qemu, "FAULT_MARK=translated_rc_native "), "native fault marker")
        require(mark,run=run,pid=pid,source=source,word="0xc0000000")
        host, channel, marked = number(mark,"host"),number(mark,"channel"),number(mark,"unix_ns")
        if channel != host & 0xfff or not start <= marked <= end:
            raise Refused("native channel/marker interval mismatch")
        result = one(records(qemu,"NATIVE_RC_RESULT "),"native channel result")
        require(result,run=run,code=32,status="0xffff",event=1,bystander=1,result="PASS")
        timestamp,message = one(xids,"native host Xid")
        prefix = f"NVRM: Xid (PCI:{pci}): 32, pid={pid}, name=kf-rc-native, channel 0x{channel:08x} "
        if not message.startswith(prefix) or timestamp < marked:
            raise Refused("native Xid identity mismatch")
        frozen = message[len(prefix):]
        if re.fullmatch(DETAIL,frozen) is None:
            raise Refused("unqualified native Xid detail")
        return {"verdict":"NATIVE_ONLY","mode":mode,"run":run,"source":source,
                "detail":frozen,"host_chid":channel,"xid_count":1}

    cfg = one(records(qemu, "RC_NEGCTL_CONFIG "), "host bench configuration")
    require(cfg, run=run, source=source)
    begin = one(records(guest, "KF_RC_PROBE_BEGIN "), "guest probe birth")
    require(begin, run=run, mode=mode, **{"class":"0xc56f"})
    owner = number(begin,"owner")
    result = one(records(guest,"KF_RC_OWNER "),"owning process result")
    require(result,run=run,pid=owner,errno=5 if mode=="on" else 0,channel_error=1 if mode=="on" else 0)
    require(one(records(guest,"RC_CONTROL_HEALTH "),"post-fault guest health"),run=run,result="PASS")
    engine = number(begin, "engine")
    if not 9 <= engine <= 18:
        raise Refused("unqualified C56F copy engine")
    chid = number(begin, "chid")
    if chid > 0xfff:
        raise Refused("guest chid beyond C56F vector")
    require(one(records(guest, "KF_RC_PROBE_END "), "guest verdict"), run=run, result="PASS")
    by = records(guest, "KF_RC_BYSTANDER ")
    before = one([r for r in by if r.get("phase")=="before"], "bystander before")
    after = one([r for r in by if r.get("phase")=="after"], "bystander after")
    require(before, run=run, value=1)
    require(after, run=run, value=2)
    if len(by) != 2:
        raise Refused("unexpected bystander record")
    markers = records(qemu, "FAULT_MARK=translated_rc_invalid_pb ")
    events = records(guest, "KF_RC_EVENT ")
    notifiers = records(guest, "KF_RC_NOTIFIER ")
    if mode == "off":
        if markers or events or notifiers or xids or "RC_NEGCTL_SUBMITTED " in qemu:
            raise Refused("clean arm contains fault evidence")
        require(one(records(guest, "KF_RC_CLEAN "), "clean notifier check"), run=run, status="0x0")
        return {"verdict":"PASS", "mode":mode, "run":run, "source":source}

    mark = one(markers, "intentional fault marker")
    require(mark, run=run, pid=pid, guest=hex(chid), engine=hex(engine), word="0xc0000000")
    host, host_chid, marked = number(mark,"host"), number(mark,"channel"), number(mark,"unix_ns")
    if host_chid != (host & 0xfff) or not start <= marked <= end:
        raise Refused("invalid host channel/marker interval")
    born = re.findall(r"BORN Translated: token (0x[0-9a-f]+) -> host (0x[0-9a-f]+)[^\n]*engine=(0x[0-9a-f]+)", qemu)
    if sum(int(a,16)==chid and int(b,16)==host and int(c,16)==engine for a,b,c in born) != 1:
        raise Refused("no unique Translated host-twin mapping")
    require(one(records(qemu, "RC_NEGCTL_SUBMITTED "), "native submission"), run=run, host=hex(host))
    seen = re.findall(r"RC Translated host (0x[0-9a-f]+) guest chid (0x[0-9a-f]+) engine (0x[0-9a-f]+) Xid ([0-9]+)",qemu)
    posted = re.findall(r"RC_TRIGGERED posted: guest chid (0x[0-9a-f]+) engine (0x[0-9a-f]+) except_type (0x[0-9a-f]+) \(host (0x[0-9a-f]+)\)",qemu)
    if seen != [(hex(host),hex(chid),hex(engine),"32")] or posted != [(hex(chid),hex(engine),"0x20",hex(host))]:
        raise Refused("host observation/posting is absent, duplicated or misrouted")
    require(one(events,"guest RC event"), chid=hex(chid), engine=hex(engine), xid=32, scope=0)
    note = one(notifiers,"guest notifier read")
    require(note,run=run,chid=hex(chid),info32=32,info16=engine,status="0xffff",get=0)
    if number(note,"timestamp") == 0:
        raise Refused("empty notifier timestamp")
    timestamp, message = one(xids,"host Xid")
    expected = f"NVRM: Xid (PCI:{pci}): 32, pid={pid}, name=qemu-system-x86, channel 0x{host_chid:08x} {detail}"
    if message != expected or timestamp < marked:
        raise Refused("host Xid differs from exact intentional control")
    return {"verdict":"PASS", "mode":mode, "run":run, "source":source,
            "host_chid":host_chid, "guest_chid":chid, "xid_count":1}


def bounded(path):
    with Path(path).open("rb") as stream:
        data = stream.read(MAX_BYTES+1)
    if len(data)>MAX_BYTES:
        raise Refused("capture exceeds byte bound")
    return data.decode("utf-8", "strict")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("manifest", "qemu", "guest", "journal"):
        parser.add_argument("--"+arg, required=True)
    args=parser.parse_args()
    try:
        result=check(json.loads(bounded(args.manifest)),bounded(args.qemu),bounded(args.guest),bounded(args.journal))
    except (Refused, OSError, ValueError) as error:
        print(json.dumps({"verdict":"FAIL","reason":str(error)}))
        return 1
    print(json.dumps(result,sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
