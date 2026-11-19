#!/usr/bin/env python3
"""Full host kernel log for a run window, with a line offset per stage start/end.

The manual rehearsal (traces/release_manual_rehearsal_20261115) read dmesg late and tailed it: 8 of 15 `Xid 31`
lines were kept, and none could be tied to a stage. This follows the kernel log from the start of the run
(`dmesg -w --time-format iso`, which prints the whole ring first; `journalctl -k -f` as a fallback) and appends
every line to DIR/kernel.log. mark() records the line count of that file at each stage start and end in
DIR/stage-offsets.tsv, so any later line number maps to a stage (attribute()).

    kernel_log.py start DIR            # detached follower; prints nothing, pid in DIR/follower.pid
    kernel_log.py mark DIR STAGE start|end
    kernel_log.py xids DIR [REGEX]     # matching lines: line number, stage, text
    kernel_log.py stop DIR

Limit (said, not hidden): a kernel line that reaches the follower after mark() settled is counted in the next
window; SETTLE_S bounds that. An unreadable log (dmesg_restrict, no CAP_SYSLOG) is an error at start, never an
empty capture.
"""
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

SETTLE_S = 1.0
XID = r"NVRM: Xid"
SOURCES = (["dmesg", "-w", "--time-format", "iso"], ["journalctl", "-k", "-f", "-o", "short-iso", "--no-pager"])


def log_path(directory):
    return Path(directory) / "kernel.log"


def offsets_path(directory):
    return Path(directory) / "stage-offsets.tsv"


def count_lines(path):
    try:
        with open(path, "rb") as f:
            return sum(chunk.count(b"\n") for chunk in iter(lambda: f.read(1 << 20), b""))
    except FileNotFoundError:
        return 0


def follow(directory, source=None):
    """Foreground loop: copy the kernel log to DIR/kernel.log line by line, flushed. Returns the child's rc."""
    command = source or next(c for c in SOURCES if subprocess.run(["which", c[0]], capture_output=True).returncode == 0)
    child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
    signal.signal(signal.SIGTERM, lambda *_: child.terminate())
    with open(log_path(directory), "a") as out:
        for line in child.stdout:
            out.write(line)
            out.flush()
    return child.wait()


def start(directory, source=None):
    """Start the detached follower; fail loudly when the log is not readable (no silent empty capture)."""
    Path(directory).mkdir(parents=True, exist_ok=True)
    command = [sys.executable, os.path.abspath(__file__), "follow", str(directory)]
    if source:
        command += source
    proc = subprocess.Popen(command, start_new_session=True, stdin=subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    (Path(directory) / "follower.pid").write_text(str(proc.pid) + "\n")
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError("kernel log follower exited rc=%s: the kernel log is not readable" % proc.returncode)
        if count_lines(log_path(directory)) > 0:
            return proc.pid
        time.sleep(0.1)
    raise RuntimeError("kernel log follower produced no lines in 5s: the run would be unwatched")


def stop(directory):
    pid_file = Path(directory) / "follower.pid"
    try:
        os.kill(int(pid_file.read_text()), signal.SIGTERM)
    except (FileNotFoundError, ProcessLookupError, ValueError):
        pass


def mark(directory, stage, edge, settle=None, clock=time.time):
    """Append `stage<TAB>edge<TAB>line_count<TAB>epoch` after letting the follower catch up."""
    if edge not in ("start", "end"):
        raise ValueError(edge)
    wait = SETTLE_S if settle is None else settle
    last = count_lines(log_path(directory))
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline:
        time.sleep(min(0.2, wait))
        now = count_lines(log_path(directory))
        if now == last:
            break
        last = now
    with open(offsets_path(directory), "a") as f:
        f.write("%s\t%s\t%d\t%d\n" % (stage, edge, last, int(clock())))
    return last


def windows(directory):
    """[(stage, first_line, last_line)] with 1-based inclusive line numbers; an open stage runs to the end."""
    opened, out = {}, []
    try:
        rows = [r.split("\t") for r in offsets_path(directory).read_text().splitlines()]
    except FileNotFoundError:
        rows = []
    for stage, edge, n, _ in rows:
        if edge == "start":
            opened[stage] = int(n)
        elif stage in opened:
            out.append((stage, opened.pop(stage) + 1, int(n)))
    out.extend((stage, n + 1, 1 << 62) for stage, n in opened.items())
    return out


def attribute(directory, line_no):
    """The stage that was running when line `line_no` (1-based) was logged, else 'outside-stages'."""
    for stage, first, last in windows(directory):
        if first <= line_no <= last:
            return stage
    return "outside-stages"


def matching(directory, pattern=XID):
    rx = re.compile(pattern)
    with open(log_path(directory), errors="replace") as f:
        return [(n, attribute(directory, n), line.rstrip("\n")) for n, line in enumerate(f, 1) if rx.search(line)]


def main(argv):
    cmd = argv[1] if len(argv) > 1 else ""
    if cmd == "follow" and len(argv) >= 3:
        return follow(argv[2], argv[3:] or None)
    if cmd == "start" and len(argv) == 3:
        print("follower pid", start(argv[2]))
        return 0
    if cmd == "mark" and len(argv) == 5:
        print(mark(argv[2], argv[3], argv[4]))
        return 0
    if cmd == "xids" and len(argv) in (3, 4):
        for n, stage, line in matching(argv[2], *argv[3:]):
            print("%d\t%s\t%s" % (n, stage, line))
        return 0
    if cmd == "stop" and len(argv) == 3:
        stop(argv[2])
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
