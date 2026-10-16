#!/usr/bin/env python3
"""qmp1.py SOCK json-command [json-command...] : send QMP commands on one connection, print returns (and the events seen)."""
import socket, json, sys
s = socket.socket(socket.AF_UNIX); s.settimeout(30); s.connect(sys.argv[1]); f = s.makefile("rwb", buffering=0); f.readline()
def q(c):
    f.write(json.dumps(c).encode() + b"\n")
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r: return r
q({"execute": "qmp_capabilities"})
for a in sys.argv[2:]: print(json.dumps(q(json.loads(a)))[:300])
