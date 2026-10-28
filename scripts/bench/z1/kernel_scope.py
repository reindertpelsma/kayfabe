"""Bounded kernel capture from a pinned journal cursor, never a moving tail.

The boot journal may contain unrelated long-running campaigns. Capture two real
kernel records before Z1, retain their first cursor and boot ID, and include every
record from that cursor onwards on every read. A lost cursor, reboot, non-kernel
record, malformed JSON or byte-bound violation refuses. The Xid oracle is unchanged.
"""

import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

MAX_BYTES = 16 * 1024 * 1024


class ScopeRefused(ValueError):
    pass


def read_records(arguments):
    child = subprocess.Popen(
        ["timeout", "10", "journalctl", "-k", "-b", "--no-pager", "-o", "json", *arguments],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        data = child.stdout.read(MAX_BYTES + 1)
        if len(data) > MAX_BYTES:
            raise ScopeRefused("kernel scope exceeded the unchanged 16 MiB capture bound")
        if child.wait(timeout=10) != 0:
            raise ScopeRefused("kernel journal reader failed")
        records = [json.loads(line) for line in data.decode("utf-8", "strict").splitlines()]
        return records, data
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=10)


def decode(records, scope):
    if not records or records[0].get("__CURSOR") != scope["cursor"]:
        raise ScopeRefused("initial kernel cursor is missing or vacuumed")
    lines = []
    for record in records:
        if record.get("_BOOT_ID") != scope["boot"]:
            raise ScopeRefused("kernel capture crosses boot identity")
        if record.get("_TRANSPORT") != "kernel":
            raise ScopeRefused("kernel transport proof missing")
        message = record["MESSAGE"]
        if not isinstance(message, str):
            raise ScopeRefused("kernel message is not strict text")
        stamp = datetime.datetime.fromtimestamp(
            int(record["__REALTIME_TIMESTAMP"]) / 1e6, datetime.timezone.utc
        ).isoformat(timespec="microseconds")
        lines.extend(f"{stamp} kernel: {line}" for line in message.splitlines())
    return "\n".join(lines) + "\n"


def create(path):
    records, raw = read_records(["-n", "2"])
    if len(records) != 2:
        raise ScopeRefused("two readable real kernel entries required to anchor capture")
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip().replace("-", "")
    scope = {
        "cursor": records[0]["__CURSOR"],
        "boot": boot,
        "initial_json_sha256": hashlib.sha256(raw).hexdigest(),
    }
    decode(records, scope)
    Path(path).write_text(json.dumps(scope, indent=2) + "\n")
    Path(str(path) + ".initial.jsonl").write_bytes(raw)
    return scope


def snapshot():
    path = Path(os.environ["KF_Z1_KERNEL_SCOPE"])
    scope = json.loads(path.read_text())
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip().replace("-", "")
    if boot != scope["boot"]:
        raise ScopeRefused("host rebooted after the kernel cursor was pinned")
    records, raw = read_records(["--cursor=" + scope["cursor"]])
    text = decode(records, scope)
    latest = Path(str(path) + ".latest.jsonl")
    temporary = latest.with_suffix(".tmp")
    temporary.write_bytes(raw)
    temporary.replace(latest)
    return text


if __name__ == "__main__":
    print(json.dumps(create(sys.argv[1])))
