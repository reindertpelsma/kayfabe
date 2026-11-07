"""Quiesce a pinned controller tree before collecting a final capture."""

import json
import os
from pathlib import Path
import signal
import sys
import time


def processes():
    result = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            fields = (entry / "stat").read_text().rpartition(")")[2].split()
            result[int(entry.name)] = (int(fields[1]), int(fields[19]))
        except (FileNotFoundError, ProcessLookupError):
            pass
    return result


def quiesce(pid, starttime, *, snapshot=processes, send=os.kill, pause=time.sleep):
    if type(pid) is not int or pid <= 1 or type(starttime) is not int or starttime <= 0:
        raise ValueError("invalid controller identity")
    initial = snapshot()
    if pid not in initial:
        return
    if initial[pid][1] != starttime:
        raise ValueError("controller PID reused")
    owned = {pid: starttime}

    def signal_if_same(target, sig):
        current = snapshot().get(target)
        if current is not None and current[1] == owned[target]:
            try:
                send(target, sig)
            except ProcessLookupError:
                pass

    signal_if_same(pid, signal.SIGSTOP)
    # Freeze parents before their children. A final snapshot catches children born before STOP.
    for _ in range(100):
        table = snapshot()
        added = []
        for child, (parent, born) in table.items():
            if parent in owned and child not in owned:
                parent_row = table.get(parent)
                if parent_row is not None and parent_row[1] == owned[parent]:
                    owned[child] = born
                    added.append(child)
        for child in added:
            signal_if_same(child, signal.SIGSTOP)
        if not added:
            break
    else:
        raise ValueError("controller tree exceeds bounded traversal")
    # KILL does not resume a stopped task or permit new work during collection.
    for target in reversed(list(owned)):
        signal_if_same(target, signal.SIGKILL)
    pause(1)
    alive = snapshot()
    # Zombies cannot mutate files; /proc existence alone does not mean running.
    for target, born in owned.items():
        if alive.get(target, (None, None))[1] != born:
            continue
        stat = Path("/proc", str(target), "stat")
        if stat.exists() and stat.read_text().rpartition(")")[2].split()[0] != "Z":
            raise ValueError("owned controller tree did not quiesce")


if __name__ == "__main__":
    pid, born = map(int, sys.argv[1:])
    quiesce(pid, born)
    print(json.dumps(dict(controller_pid=pid, controller_starttime=born, quiesced=True)))
