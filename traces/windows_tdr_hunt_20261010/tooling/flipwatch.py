#!/usr/bin/env python3
"""Host-side watcher (TDR hunt, shape F): tail a kf3 qemu.log with KF3_DISPLAY_WRITE_TRACE. A window LATCH whose vblank the
guest acked at/after the latch (ack - latch > -0.15 ms, or no ack in that frame) and after which no window PUT comes for HOLD
while flips were flowing is the stuck-flip signature (runs 268-279): dump guest memory once via QMP, write the evidence to OUT.
usage: flipwatch.py qemu.log qmp.sock dump.elf out.txt [hold_ms] [--replay]"""
import json, re, socket, sys, time
args = [a for a in sys.argv[1:] if a != "--replay"]; replay = "--replay" in sys.argv
log, qmp, dump, out = args[:4]
hold = float(args[4]) / 1000 if len(args) > 4 else 0.4
pat = re.compile(r"WTRACE t=([0-9.]+) (.*)")

def qmp_cmd(cmds):
    s = socket.socket(socket.AF_UNIX); s.connect(qmp); f = s.makefile("rw")
    f.readline(); f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n"); f.flush(); f.readline()
    res = []
    for c in cmds:
        f.write(json.dumps(c) + "\n"); f.flush()
        while True:
            l = json.loads(f.readline())
            if "return" in l or "error" in l: res.append(l); break
    s.close(); return res

recent = []
last_put = None; puts = 0; vs = None; ack = None; pend = None  # pend = [latch_t, ack_t or None, puts, late]
f = open(log, errors="replace")
if not replay: f.seek(0, 2)
start = time.time()
while True:
    ln = f.readline()
    if not ln:
        if replay: break
        time.sleep(0.02); continue
    m = pat.search(ln)
    if not m: continue
    t, what = float(m.group(1)), m.group(2)
    recent.append(f"{t:.6f} {what[:170]}"); recent = recent[-500:]
    if "WRITE 0x690000" in what:
        last_put = t; puts += 1
    elif "WRITE 0x611800" in what:
        if ack is None:
            ack = t
            if pend and pend[1] is None and pend[3] is None:
                pend[1] = t; pend[3] = (t - pend[0]) > -0.00015
    elif "LATCH window 0" in what:
        pend = [t, ack, puts, None if ack is None else (ack - t) > -0.00015]
    elif "VSYNC" in what:
        if pend and pend[3] is None:
            pend[3] = True  # no ack in the latch's frame
        if pend and pend[3] and last_put is not None and pend[0] > last_put and t - last_put > hold \
                and pend[2] >= 20 and (replay or time.time() - start > 20):
            if replay:
                print(f"FIRE at {t:.6f}: stuck latch {pend[0]:.6f} ack {pend[1]}"); pend = None; continue
            with open(out, "w") as o:
                o.write(f"STUCK flip: latch {pend[0]:.6f} ack {pend[1]} last PUT {last_put:.6f} now {t:.6f}\n" + "\n".join(recent) + "\n")
            r = qmp_cmd([{"execute": "dump-guest-memory", "arguments": {"paging": False, "protocol": "file:" + dump, "detach": True}}])
            with open(out, "a") as o: o.write(f"dump requested {time.time():.3f}: {r}\n")
            for _ in range(120):
                time.sleep(2)
                st = qmp_cmd([{"execute": "query-dump"}])
                if "completed" in json.dumps(st) or "failed" in json.dumps(st):
                    with open(out, "a") as o: o.write(f"dump status: {st}\n")
                    break
            sys.exit(0)
        vs = t; ack = None
