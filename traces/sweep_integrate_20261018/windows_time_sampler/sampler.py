"""Rental-only bounded metadata snapshots; sample times are observation bounds."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import time

RPC = re.compile(rb"kf-rm: rpc-trace fn=(\d+) (.*?) seq=(\d+) (.*?) result=(\S+)")


def snapshot(data, sample_utc, *, max_bytes=64 * 1024 * 1024, max_commands=16384):
    if len(data) > max_bytes:
        raise ValueError("source byte quota exceeded")
    commands = []
    for line in data.splitlines(keepends=True):
        if not line.endswith(b"\n"):
            continue
        match = RPC.search(line)
        if not match:
            continue
        if len(commands) >= max_commands:
            raise ValueError("metadata quota exceeded")
        fn, _kind, seq, what, result = match.groups()
        selector = re.search(rb"(?:cmd|class)=(0x[0-9a-fA-F]+)", what)
        commands.append({"ordinal": len(commands), "function": int(fn),
                         "rpc_sequence": int(seq),
                         "selector": selector[1].decode().lower() if selector else None,
                         "trace_result": result.decode(errors="replace")})
    return {"sample_utc": sample_utc, "source_bytes": len(data),
            "source_sha256": hashlib.sha256(data).hexdigest(),
            "commands": len(commands), "last": commands[-1] if commands else None,
            "unload_ordinals": [r["ordinal"] for r in commands if r["function"] == 47],
            "display_class_ordinals": [r["ordinal"] for r in commands
                                       if r["selector"] in ("0xca70", "0xca7a")],
            "metadata_quota_notice": b"rpc-trace lifetime limit reached" in data,
            "meaning": "observation upper bound; RPC policy metadata, not effective status"}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--seconds", type=int, default=900)
    parser.add_argument("--interval", type=int, default=10)
    args = parser.parse_args()
    if not 1 <= args.seconds <= 1800 or not 1 <= args.interval <= 30:
        parser.error("bounded window/interval required")
    deadline = time.monotonic() + args.seconds
    with args.output.open("x") as out:
        while time.monotonic() < deadline:
            if args.source.exists():
                with args.source.open("rb") as source:
                    data = source.read(64 * 1024 * 1024 + 1)
                row = snapshot(data, datetime.datetime.now(datetime.timezone.utc).isoformat())
                out.write(json.dumps(row, sort_keys=True) + "\n")
                out.flush()
            time.sleep(min(args.interval, max(0, deadline - time.monotonic())))


if __name__ == "__main__":
    main()
