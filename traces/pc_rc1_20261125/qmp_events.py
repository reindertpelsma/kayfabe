#!/usr/bin/env python3
"""Records every QMP event of one QEMU on its own QMP socket as JSON lines, until the socket closes.
usage: qmp_events.py <qmp-socket> <out.jsonl>"""
import json
import socket
import sys
import time


def main():
    path, out = sys.argv[1], sys.argv[2]
    s = None
    for _ in range(100):
        try:
            s = socket.socket(socket.AF_UNIX)
            s.connect(path)
            break
        except OSError:
            s.close()
            s = None
            time.sleep(0.2)
    with open(out, "a", buffering=1) as o:
        if s is None:
            o.write(json.dumps({"recorder": "never connected"}) + "\n")
            return
        f = s.makefile("rw")
        f.readline()  # greeting
        f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n")
        f.flush()
        o.write(json.dumps({"recorder": "connected", "t": time.time()}) + "\n")
        for line in f:
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if "event" in m:
                m["t_host"] = time.time()
                o.write(json.dumps(m) + "\n")
        o.write(json.dumps({"recorder": "socket closed", "t": time.time()}) + "\n")


main()
