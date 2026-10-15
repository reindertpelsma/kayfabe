#!/usr/bin/env python3
"""bootshots.py QMP_SOCK OUTDIR [SECONDS=25] [HZ=8] : one persistent QMP connection, QMP screendump series for the first SECONDS after the
socket appears (boot/UEFI frames). Keeps a frame only when its bytes differ from the previous kept one. Passive; does not touch the guest."""
import socket, json, sys, time, os, hashlib
sock, out = sys.argv[1], sys.argv[2]; secs = float(sys.argv[3]) if len(sys.argv) > 3 else 25; hz = float(sys.argv[4]) if len(sys.argv) > 4 else 8
os.makedirs(out, exist_ok=True)
t_end0 = time.time() + 120
while not os.path.exists(sock):
    if time.time() > t_end0: sys.exit("no socket")
    time.sleep(0.05)
s = socket.socket(socket.AF_UNIX); s.settimeout(10)
for _ in range(200):
    try: s.connect(sock); break
    except OSError: time.sleep(0.05)
f = s.makefile("rwb", buffering=0); f.readline()
def q(c, a=None):
    m = {"execute": c}
    if a: m["arguments"] = a
    f.write(json.dumps(m).encode() + b"\n")
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r: return r
q("qmp_capabilities")
t0 = time.time(); n = 0; kept = 0; last = None; log = open(os.path.join(out, "index.txt"), "w")
while time.time() - t0 < secs:
    tt = time.time(); p = os.path.join(out, "%03d.ppm" % n)
    r = q("screendump", {"filename": p})
    if "error" in r: log.write("%03d t=%.2f error %s\n" % (n, tt - t0, r["error"].get("desc", "")[:80])); log.flush()
    elif os.path.exists(p):
        h = hashlib.md5(open(p, "rb").read()).hexdigest()
        if h == last: os.remove(p)
        else: last = h; kept += 1; log.write("%03d t=%.2f md5=%s size=%d\n" % (n, tt - t0, h[:8], os.path.getsize(p))); log.flush()
    n += 1
    time.sleep(max(0, 1 / hz - (time.time() - tt)))
log.write("DONE shots=%d kept=%d\n" % (n, kept)); log.close()
