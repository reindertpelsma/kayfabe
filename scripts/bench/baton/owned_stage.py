#!/usr/bin/env python3
"""Keep the resource flock outside the owned-child guardian and its cleanup."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from hw2_session import monitor_owned, xid_count


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lock", required=True)
    ap.add_argument("--seconds", type=float, required=True)
    ap.add_argument("--baseline", type=int, required=True)
    ap.add_argument("--result", type=Path, required=True)
    ap.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    ap.add_argument("command", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not args.inside:
        return subprocess.call(["flock", "-w", "50", "-o", args.lock,
                                sys.executable, str(Path(__file__).resolve()), "--inside"] + sys.argv[1:])
    parent = Path(f"/proc/{os.getppid()}/cmdline").read_bytes().decode().strip("\0").split("\0")
    probe = subprocess.run(["flock", "-n", args.lock, "true"])
    if Path(parent[0]).name != "flock" or parent[1:5] != ["-w", "50", "-o", args.lock] or probe.returncode == 0:
        raise RuntimeError("owned stage must run inside its resource flock")
    proc = subprocess.Popen(command)
    reason, rc = monitor_owned(proc, args.seconds, xid_count, args.baseline, lambda: None)
    args.result.write_text(json.dumps(dict(reason=reason, rc=rc, at=time.time())))
    return int(reason != "exit" or rc != 0)


if __name__ == "__main__":
    sys.exit(main())
