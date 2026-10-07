#!/usr/bin/env python3
"""qmp_shot.py QMP_SOCKET FILE [DEVICE] — a QEMU screendump over QMP (the v3-maxfps box stage's
workaround for finding F1: an HMP `screendump` deadlocks kf3 at e7a82046 — HMP waits for a coroutine
command in a nested aio_poll of the main AioContext, which never runs kf3's refresh fd handler nor its
backstop timer). QMP's dispatcher yields to the main loop, so the device's answer is serviced.
Prints one line: screendump_ms=<n> result=<return|error ...>."""
import json, socket, sys, time
path, out = sys.argv[1], sys.argv[2]
dev = sys.argv[3] if len(sys.argv) > 3 else "kf0"
s = socket.socket(socket.AF_UNIX); s.connect(path); s.settimeout(20)
f = s.makefile("rwb", buffering=0)
def rd():
    while True:
        line = f.readline()
        if not line:
            raise EOFError("closed")
        m = json.loads(line)
        if "event" in m:
            continue
        return m
rd()  # greeting
f.write(b'{"execute":"qmp_capabilities"}\n'); rd()
t0 = time.time()
f.write(json.dumps({"execute": "screendump", "arguments": {"filename": out, "device": dev}}).encode() + b"\n")
try:
    r = rd()
    res = "return" if "return" in r else "error %s" % r.get("error")
except Exception as e:
    res = "no-answer %s" % e
print("screendump_ms=%d result=%s" % ((time.time() - t0) * 1000, res))
