#!/usr/bin/env python3
"""rwatch.py RUN_DIR QPID OUT_DIR [SLOTS] — follow kayfabe's qemu.log; for every new window-notifier page (the
`SLOT REQ ... chn 1 ... notify ... +0x0 at Sysmem PA` line that opens each driver life), find its kernel VA(s) with
revmap.py (no VM stop), then (re)start gdbwatch.py with access watchpoints on the chosen slots. Host-only diagnosis.
SLOTS: comma list of slot offsets in the page (default 0xf40,0xf48,0xf80,0xf88 = the stuck flip's 16 bytes and the one
before it); with two VAs, 0xf40/0xf48 on each."""
import os, re, signal, subprocess, sys, time, json, socket

RUN, QPID, OUT = sys.argv[1], int(sys.argv[2]), sys.argv[3]
SLOTS = [int(x, 0) for x in (sys.argv[4] if len(sys.argv) > 4 else "0xf40,0xf48,0xf80,0xf88").split(",")]
HERE = os.path.dirname(os.path.abspath(__file__))
LOG = open(os.path.join(OUT, "rwatch.log"), "a", buffering=1)
PORT = 1234


def say(s):
    LOG.write("%.6f %s\n" % (time.time(), s))


def hmp(cmd):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(10)
    s.connect(os.path.join(RUN, "qmp.sock"))
    f = s.makefile("rwb", buffering=0)
    f.readline()
    f.write(b'{"execute":"qmp_capabilities"}\n'); f.readline()
    f.write(json.dumps({"execute": "human-monitor-command", "arguments": {"command-line": cmd}}).encode() + b"\n")
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            s.close()
            return r.get("return", str(r.get("error")))


def lowmem():
    for ln in hmp("info mtree").splitlines():
        m = re.search(r"([0-9a-f]+)-([0-9a-f]+) \(prio [-0-9]+, ram\): alias ram-below-4g", ln)
        if m:
            return int(m.group(2), 16) + 1
    return 0xC0000000


def kernel_cr3s():
    regs = hmp("info registers -a")
    out = []
    for blk in regs.split("CPU#")[1:]:
        if "CPL=0" in blk:
            m = re.search(r"CR3=([0-9a-f]+)", blk)
            if m and int(m.group(1), 16) & ~0xFFF not in out:
                out.append(int(m.group(1), 16) & ~0xFFF)
    return out


def ram_path():
    best = None
    for fd in os.listdir("/proc/%d/fd" % QPID):
        try:
            t = os.readlink("/proc/%d/fd/%s" % (QPID, fd))
        except OSError:
            continue
        if t.startswith("/memfd:") and "kf3-" not in t and "display" not in t:
            p = "/proc/%d/fd/%s" % (QPID, fd)
            if best is None or os.stat(p).st_size > os.stat(best).st_size:
                best = p
    return best


def main():
    say("start: " + hmp("gdbserver tcp:127.0.0.1:%d" % PORT).strip())
    LM = lowmem()
    global RAM
    RAM = ram_path()
    say("ram=%s" % RAM)
    say("lowmem=%#x" % LM)
    qlog = os.path.join(RUN, "qemu.log")
    pos, seen, gdbp = 0, set(), None
    pat = re.compile(r"SLOT REQ .* chn 1 .*notify ctxdma (0x[0-9a-f]+) \+0x0 at Sysmem (0x[0-9a-f]+)")
    while True:
        try:
            os.kill(QPID, 0)
        except OSError:
            break
        with open(qlog, "rb") as f:
            f.seek(pos)
            data = f.read()
            pos += len(data)
        for ln in data.decode("latin-1").splitlines():
            m = pat.search(ln)
            if not m:
                continue
            page = int(m.group(2), 16) & ~0xFFF
            if page in seen:
                continue
            seen.add(page)
            t0 = time.time()
            cr3s = kernel_cr3s()
            vas = []
            for cr3 in cr3s:
                r = subprocess.run([os.path.join(HERE, "revmap"), RAM, hex(cr3), hex(page), hex(LM)],
                                   capture_output=True, text=True, timeout=30)
                vas += [int(x, 16) for x in r.stdout.split() if int(x, 16) not in vas]
            say("page %#x cr3s=%s vas=%s (%.2f s)" % (page, [hex(c) for c in cr3s], [hex(v) for v in vas],
                                                      time.time() - t0))
            if not vas:
                continue
            if gdbp and gdbp.poll() is None:
                gdbp.send_signal(signal.SIGINT)
                try:
                    gdbp.wait(10)
                except subprocess.TimeoutExpired:
                    say("gdb did not exit on SIGINT")
            if len(vas) >= 2:
                addrs = ["%s+%#x:%#x" % (chr(65 + i), s, v + s) for i, v in enumerate(vas[:2]) for s in SLOTS[:2]]
            else:
                addrs = ["A+%#x:%#x" % (s, vas[0] + s) for s in SLOTS[:4]]
            env = dict(os.environ, GW_PORT=str(PORT), GW_ADDRS=",".join(addrs), GW_LOG=os.path.join(OUT, "hits.log"))
            gdbp = subprocess.Popen(["gdb", "-q", "-nx", "-batch", "-x", os.path.join(HERE, "gdbwatch.py")], env=env,
                                    stdout=open(os.path.join(OUT, "gdb.out"), "ab"), stderr=subprocess.STDOUT)
            say("gdb pid %d watching %s" % (gdbp.pid, addrs))
        if os.path.exists(os.path.join(OUT, ".rwatch_stop")):
            break
        time.sleep(0.05)
    if gdbp and gdbp.poll() is None:
        gdbp.send_signal(signal.SIGINT)
        try:
            gdbp.wait(10)
        except subprocess.TimeoutExpired:
            gdbp.kill()
    say("end")


if __name__ == "__main__":
    main()
