#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""irq_sampler.py — DIAGNOSTIC (TDR hunt, 2026-10-10): sample the guest's interrupt-delivery state through QMP/HMP while the
guest runs. usage: irq_sampler.py QMP_SOCK QEMU_PID OUT.txt STOPFILE [PERIOD_S] [NCPU]
Each sample (one short QMP connection): `info lapic <id>` for every vCPU (IRR/ISR/TPR/PPR lines kept), the MSI-X table entries 0-3
and the PBA of the kf3 device (BAR5, found through `info pci`), `info registers -a` RFLAGS/RIP/CS-CPL per vCPU, and the eventfd
counts of the QEMU process (an unconsumed count on the device's irq eventfd means KVM's irqfd is not injecting it). Reads only."""
import json, os, re, socket, sys, time

def qmp(path):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(5); s.connect(path)
    f = s.makefile("rwb", buffering=0); json.loads(f.readline())
    f.write(b'{"execute":"qmp_capabilities"}\n'); json.loads(f.readline())
    return s, f

def hmp(f, cmd):
    f.write(json.dumps({"execute": "human-monitor-command", "arguments": {"command-line": cmd}}).encode() + b"\n")
    while True:
        r = json.loads(f.readline())
        if "return" in r: return r["return"]
        if "error" in r: return "ERR " + str(r["error"])

def eventfds(pid):
    out = {}
    try:
        for fd in os.listdir("/proc/%d/fd" % pid):
            try:
                if os.readlink("/proc/%d/fd/%s" % (pid, fd)) != "anon_inode:[eventfd]": continue
                for ln in open("/proc/%d/fdinfo/%s" % (pid, fd)):
                    if ln.startswith("eventfd-count"):
                        n = int(ln.split()[1], 16)
                        if n: out[int(fd)] = n
            except OSError: pass
    except OSError: pass
    return out

def main():
    sock, pid, outp, stop = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
    period = float(sys.argv[5]) if len(sys.argv) > 5 else 0.25
    ncpu = int(sys.argv[6]) if len(sys.argv) > 6 else 8
    out = open(outp, "a", buffering=1)
    bar5 = None
    while not os.path.exists(stop):
        t0 = time.time()
        try:
            s, f = qmp(sock)
            if bar5 is None:
                m = re.search(r"device\s+6, function 0:.*?BAR5: \d+ bit memory at (0x[0-9a-f]+)", hmp(f, "info pci"), re.S)
                if m: bar5 = int(m.group(1), 16); out.write("BAR5=%#x\n" % bar5)
            out.write("== sample utc_ms=%d\n" % int(time.time() * 1000))
            for c in range(ncpu):
                r = hmp(f, "info lapic %d" % c)
                keep = [l.strip() for l in r.replace("\r", "").split("\n") if re.search(r"^(IRR|ISR|TMR|TPR|PPR|APR|Timer|SPIV|ESR)|^\s*\d+\s*:|CPU", l)]
                out.write("cpu%d " % c + " | ".join(keep)[:900] + "\n")
            if bar5:
                out.write("msix " + " ".join(hmp(f, "xp /16wx %#x" % bar5).replace("\r", "").split("\n")[:6]) + "\n")
                out.write("pba " + " ".join(hmp(f, "xp /2wx %#x" % (bar5 + 0x2000)).replace("\r", "").split("\n")[:2]) + "\n")
            regs = hmp(f, "info registers -a").replace("\r", "")
            for m in re.finditer(r"CPU#(\d+).*?RIP=([0-9a-f]+) RFL=([0-9a-f]+).*?CPL=(\d).*?HLT=(\d)", regs, re.S):
                out.write("reg cpu%s rip=%s IF=%d cpl=%s hlt=%s\n" % (m.group(1), m.group(2), (int(m.group(3), 16) >> 9) & 1, m.group(4), m.group(5)))
            out.write("eventfd " + json.dumps(eventfds(pid)) + "\n")
            s.close()
        except Exception as e:  # QEMU gone or busy: keep trying until the stop file
            out.write("ERR %r\n" % (e,))
        time.sleep(max(0.0, period - (time.time() - t0)))

main()
