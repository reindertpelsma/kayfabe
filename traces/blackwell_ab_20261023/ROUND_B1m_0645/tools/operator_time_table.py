"""Offline time table for one exported arm (B1m evidence item d): guest vs host time.

Inputs (all already exported by export_arm.py): collect_<run>.log (CLOCK host/guest line at the
collection boot), arm_<run>_enable.txt (guest enable times), dump_headers.jsonl (guest system
time of each dump), native_receipt_observation_context.json (host kernel time of each admitted
Xid), rpc_timing.jsonl and summary.json (host observer time per RPC ordinal).
The guest-to-host offset is measured once at the collection boot and assumed constant across
the arm (INFERRED, stated in the output). Usage: time_table.py <arm_dir> <run> <out.json>
"""

import datetime as dt
import json
from pathlib import Path
import re
import sys


def parse_utc(text):
    text = text.strip().rstrip("Z")
    if "+" in text[10:]:
        text = text[: text.rindex("+")]
    head, _, frac = text.partition(".")
    stamp = dt.datetime.strptime(head, "%Y-%m-%dT%H:%M:%S").replace(tzinfo=dt.timezone.utc)
    if frac:
        stamp += dt.timedelta(microseconds=int((frac + "000000")[:6]))
    return stamp


def rows(path):
    out = []
    for line in Path(path).read_text().splitlines():
        if line.startswith("{"):
            out.append(json.loads(line))
    return out


def offset_seconds(collect_log):
    m = re.search(r"CLOCK host=(\S+) guest=(\S+)", Path(collect_log).read_text())
    if not m:
        raise ValueError("no CLOCK line")
    return (parse_utc(m[2]) - parse_utc(m[1])).total_seconds()


def nearest_ordinals(rpc, host_time, window=2.0):
    near = [r for r in rpc if abs((parse_utc(r["observed_utc"]) - host_time).total_seconds())
            <= window]
    if not near:
        return None
    return {"first": near[0]["ordinal"], "last": near[-1]["ordinal"], "count": len(near),
            "window_s": window}


def build(arm, run):
    arm = Path(arm)
    off = offset_seconds(arm / ("collect_%s.log" % run))
    rpc = rows(arm / "rpc_timing.jsonl") if (arm / "rpc_timing.jsonl").exists() else []
    by_ord = {r["ordinal"]: r for r in rpc}
    events = []

    def add(name, guest=None, host=None, source=""):
        if guest is not None and host is None:
            host = guest - dt.timedelta(seconds=off)
        if host is not None and guest is None:
            guest = host + dt.timedelta(seconds=off)
        events.append({"event": name, "host_utc": host.isoformat(), "guest_utc": guest.isoformat(),
                       "source": source, "rpc_near": nearest_ordinals(rpc, host)})

    enable = (arm / ("arm_%s_enable.txt" % run)).read_text()
    for key, name in (("ENABLE580_START", "enable start"), ("ENABLE580_OBSERVED", "Code 0 observed")):
        m = re.search(key + r".*?UTC=(\S+?);?\s", enable + " ")
        if m:
            add(name, guest=parse_utc(m[1]), source="arm_%s_enable.txt (guest clock)" % run)
    for h in rows(arm / "dump_headers.jsonl"):
        if h.get("bugcheck") and h.get("system_time_utc"):
            add("dump bugcheck " + h["bugcheck"], guest=parse_utc(h["system_time_utc"]),
                source="dump_headers.jsonl %s (guest clock)" % Path(h.get("file", "")).name)
    ctx = json.loads((arm / "native_receipt_observation_context.json").read_text())
    for r in ctx.get("records", []):
        line = r["raw_xid"]
        ch = re.search(r"channel (0x[0-9a-f]+)", line)
        add("Xid 32 channel " + (ch[1] if ch else "?"), host=parse_utc(line.split()[0]),
            source="host kernel log (host clock)")
    summary = json.loads((arm / "summary.json").read_text())
    for key in ("teardown_start_ordinal",):
        o = summary.get(key)
        if o in by_ord:
            add("RPC " + key + " %d" % o, host=parse_utc(by_ord[o]["observed_utc"]),
                source="rpc_timing.jsonl (host observer clock)")
    for o in summary.get("unload_ordinals", []):
        if o in by_ord:
            add("RPC unload ordinal %d" % o, host=parse_utc(by_ord[o]["observed_utc"]),
                source="rpc_timing.jsonl (host observer clock)")
    events.sort(key=lambda e: e["host_utc"])
    return {"run": run, "guest_minus_host_seconds": off,
            "offset_note": "measured once at the collection boot (CLOCK line); assumed constant "
                           "for the arm (INFERRED)",
            "events": events}


def main():
    arm, run, out = sys.argv[1], sys.argv[2], Path(sys.argv[3])
    table = build(arm, run)
    out.write_text(json.dumps(table, indent=2) + "\n")
    for e in table["events"]:
        print(e["host_utc"][11:26], "host |", e["guest_utc"][11:26], "guest |", e["event"],
              "| rpc", e["rpc_near"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
