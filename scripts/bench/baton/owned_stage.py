#!/usr/bin/env python3
"""Extracted native-stage driver; preserve the queued wrapper before lease tests."""
import argparse
import json
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
    ap.add_argument("command", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    proc = subprocess.Popen(["flock", "-w", "50", "-o", args.lock] + command)
    reason, rc = monitor_owned(proc, args.seconds, xid_count, args.baseline, lambda: None)
    args.result.write_text(json.dumps(dict(reason=reason, rc=rc, at=time.time())))
    return int(reason != "exit" or rc != 0)


if __name__ == "__main__":
    sys.exit(main())
