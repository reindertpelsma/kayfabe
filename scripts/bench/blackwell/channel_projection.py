"""Project public RPC identities, birth metadata and bounded host-clock event windows."""

import argparse
from bisect import bisect_right
import datetime as dt
import hashlib
import json
from pathlib import Path
import re

ALLOC = re.compile(
    r"rpc-trace fn=103 RmAlloc seq=\d+ class=(0x[0-9a-f]+) "
    r"client=(0x[0-9a-f]+) parent=(0x[0-9a-f]+) handle=(0x[0-9a-f]+).*?result=(\S+)"
)
BIRTH = re.compile(
    r"chan (0x[0-9a-f]+):(0x[0-9a-f]+) BORN (Translated|Passthrough): "
    r"token (0x[0-9a-f]+) -> host (0x[0-9a-f]+).*?engine=(0x[0-9a-f]+)"
)
XID = re.compile(r"NVRM: Xid .*?pid=(\d+).*?channel (0x[0-9a-f]+)")


def rpc_window(rows, at_utc):
    end = dt.datetime.fromisoformat(at_utc)
    start = end - dt.timedelta(seconds=40)
    return [r for r in rows if start <= dt.datetime.fromisoformat(r["observed_utc"]) <= end]


def project(raw, rows, channels):
    offsets = [r["file_offset_after"] for r in rows]
    ordinal_at_end = {r["file_offset_after"]: r["ordinal"] for r in rows}
    allocations, births, rc = [], [], []
    offset = 0
    for line in raw.splitlines(keepends=True):
        start = offset
        offset += len(line)
        if len(line) > 4096:
            continue
        text = line.decode("utf-8", "replace")
        if "reply-body" in text or "body=" in text:
            continue
        allocation = ALLOC.search(text)
        if allocation:
            allocations.append(
                {
                    "class": allocation[1],
                    "client": allocation[2],
                    "parent": allocation[3],
                    "handle": allocation[4],
                    "result": allocation[5],
                    "offset": start,
                    "ordinal": ordinal_at_end.get(offset),
                }
            )
        birth = BIRTH.search(text)
        if birth:
            host = int(birth[5], 16)
            # Offline candidate join only: public GB202 doorbell runlist/vector fields.
            # The Xid high-byte join remains INFERRED; the product's token is untouched.
            runlist, chid = (host >> 16) & 0x7F, host & 0xFFF
            index = bisect_right(offsets, start)
            births.append(
                {
                    "client": birth[1],
                    "handle": birth[2],
                    "route": birth[3],
                    "guest_token": birth[4],
                    "host_token": birth[5],
                    "declared_engine": birth[6],
                    "offset": start,
                    "candidate_xid_channel": hex((runlist << 24) | chid),
                    "rpc_before": rows[index - 1]["ordinal"] if index else None,
                    "rpc_after": rows[index]["ordinal"] if index < len(rows) else None,
                }
            )
        if any(word in text for word in ("RC_TRIGGERED", "TRANSLATED_RC", "RC-SEEN")):
            rc.append({"offset": start, "text": text.strip()[:600]})
    for birth in births:
        same = [
            a
            for a in allocations
            if a["client"] == birth["client"]
            and a["handle"] == birth["handle"]
            and a["result"] == "0x0"
        ]
        before = [a for a in same if a["offset"] <= birth["offset"]]
        allocation = before[-1] if before else (same[0] if same else None)
        birth["allocation_ordinal"] = allocation["ordinal"] if allocation else None
        birth["channel_class"] = allocation["class"] if allocation else None
        birth["engine_object_classes"] = [
            {"class": a["class"], "ordinal": a["ordinal"]}
            for a in allocations
            if a["client"] == birth["client"]
            and a["parent"] == birth["handle"]
            and a["result"] == "0x0"
        ]
    return {
        "allocations": allocations,
        "births": births,
        "rc_events": rc,
        "join_classification": "INFERRED",
        "join_limit": "Candidate Xid/token association; retain all reused-ID candidates.",
        "token_layout_source": "ogkm307159f2623d: kernel_fifo_gb202.c:45-85/dev_vm.h",
        "target_candidates": {
            channel: [i for i, b in enumerate(births) if b["candidate_xid_channel"] == channel]
            for channel in channels
        },
    }


def read_bounded(path, limit):
    with Path(path).open("rb") as source:
        raw = source.read(limit + 1)
    if len(raw) > limit:
        raise ValueError("evidence source exceeds its byte bound")
    return raw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("log", "rpc", "out", "kernel", "qmp", "pid"):
        parser.add_argument(name)
    args = parser.parse_args()
    raw = read_bounded(args.log, 64 * 1024 * 1024)
    rpc = read_bounded(args.rpc, 16 * 1024 * 1024)
    rows = [json.loads(line) for line in rpc.decode().splitlines()]
    if len(rows) > 32768 or any(r["ordinal"] != i for i, r in enumerate(rows)):
        raise ValueError("canonical RPC prefix is missing or outside bound")
    anchors, channels = [], set()
    for line in read_bounded(args.kernel, 4 * 1024 * 1024).decode().splitlines():
        match = XID.search(line)
        if match and int(match[1]) == int(args.pid):
            at = dt.datetime.strptime(line.split()[0], "%Y-%m-%dT%H:%M:%S.%f%z")
            anchors.append({"event": "host_xid", "utc": at.isoformat()})
            channels.add(hex(int(match[2], 16)))
    for line in read_bounded(args.qmp, 65536).decode().splitlines():
        if "GUEST_PANICKED" not in line or "{" not in line:
            continue
        event = json.loads(line[line.index("{") :])
        stamp = event["timestamp"]
        at = dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc) + dt.timedelta(
            seconds=stamp["seconds"], microseconds=stamp["microseconds"]
        )
        anchors.append({"event": "qmp_guest_panicked", "utc": at.isoformat()})
    observer = json.loads(read_bounded(Path(args.rpc).with_suffix(".receipt.json"), 65536))
    stream_complete = (
        observer.get("reason") in ("qemu_exited_at_eof", "stop_after_eof")
        and observer.get("complete_prefix") is True
        and observer.get("reached_eof") is True
        and observer.get("source_bytes_read") == len(raw)
    )
    out = project(raw, rows, sorted(channels))
    out.update(
        {
            "qemu_log_sha256": hashlib.sha256(raw).hexdigest(),
            "rpc_rows": len(rows),
            "canonical_stream_sha256": hashlib.sha256(rpc).hexdigest(),
            "observer_receipt": observer,
            "pre_event_windows_complete": stream_complete,
            "clock_basis": (
                "host journal/QMP and host-observed RPC times; guest clock not substituted"
            ),
            "pre_event_windows": [
                {
                    **anchor,
                    "seconds": 40,
                    "ordinals": [row["ordinal"] for row in rpc_window(rows, anchor["utc"])],
                }
                for anchor in anchors
            ],
        }
    )
    Path(args.out).write_text(json.dumps(out, indent=2) + "\n")


if __name__ == "__main__":
    main()
