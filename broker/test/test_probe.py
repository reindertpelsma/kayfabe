#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""The probe window against a REAL compositor: mutter --headless (software
renderer, no GPU) and real udmabuf dma-bufs.

Why this exists.  The first frame of a layout the broker has not imported goes
through an asynchronous probe (so a refusal is an event, not a dead connection).
The old code dropped every frame that arrived while the probe was out -- on a
reconnect that is the whole connect burst -- showed nothing until the guest
redrew, and never said a word about them, which the VMM's acknowledgement
detector reads as "this display cannot import dma-bufs" (MEASURED 2026-10-17 in
run 3001: three connections, three false trips).  Now the NEWEST such frame is
imported the moment its layout is proven, and every older one is released at
once ("the display is no longer reading that buffer").

Asserted, over the real socket:
  * a burst of 3 frames at connect: RELEASE for the first two within a second,
    NONE for the third (it is on screen) until a later frame replaces it, then
    exactly that RELEASE;
  * the log says the kept frame is shown;
  * a single frame (the reconnect replay) is shown, and not released early.

Skips (exit 77) when mutter, dbus-run-session or /dev/udmabuf is missing.
Needs no pointer and no pipewire: it reads the broker's own protocol events.
"""
import fcntl
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_cursor as tc   # noqa: E402
import test_xor as tx      # noqa: E402

W, H = 640, 360
STRIDE = W * 4


def skip(why):
    print("SKIP: " + why)
    sys.exit(77)


def mkbuf(gen):
    total = 4096 * ((STRIDE * H + 4095) // 4096)
    dev = os.open("/dev/udmabuf", os.O_RDWR | os.O_CLOEXEC)
    mfd = os.memfd_create("probe-frame", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    os.write(mfd, tx.frame_bytes(W, H, STRIDE, 0, gen=gen, total=total))
    fcntl.fcntl(mfd, tc.F_ADD_SEALS, tc.F_SEAL_SHRINK)
    arg = bytearray(struct.pack("<IIQQ", mfd, 1, 0, total))
    fd = fcntl.ioctl(dev, tc.UDMABUF_CREATE, arg, True)
    os.close(mfd)
    os.close(dev)
    return fd


class Session:
    def __init__(self, broker, tmp, name):
        self.broker = broker
        self.xdg = os.path.join(tmp, "xdg")
        os.makedirs(self.xdg, mode=0o700, exist_ok=True)
        self.log_path = os.path.join(tmp, name + ".log")

    def start_compositor(self):
        env = dict(os.environ, XDG_RUNTIME_DIR=self.xdg, GIO_USE_VFS="local")
        self.mutter = subprocess.Popen(
            ["dbus-run-session", "--", "mutter", "--headless", "--wayland",
             "--no-x11", "--wayland-display", "wl-probe", "--virtual-monitor",
             "1024x768"], env=env, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, start_new_session=True)
        for _ in range(100):
            if os.path.exists(os.path.join(self.xdg, "wl-probe")):
                return True
            time.sleep(0.1)
        return False

    def stop_compositor(self):
        try:
            os.killpg(self.mutter.pid, signal.SIGTERM)
        except OSError:
            pass

    def start_broker(self):
        env = dict(os.environ, XDG_RUNTIME_DIR=self.xdg, WAYLAND_DISPLAY="wl-probe",
                   ASAN_OPTIONS="detect_leaks=0")
        self.sock = os.path.join(self.xdg, "b.sock")
        if os.path.exists(self.sock):
            os.unlink(self.sock)
        self.log = open(self.log_path, "w")
        self.proc = subprocess.Popen(
            [self.broker, "--socket", self.sock, "--backend", "wayland",
             "--verbose", "--persist", "--size", "640x360"], env=env,
            stderr=self.log, stdout=self.log, stdin=subprocess.DEVNULL)
        for _ in range(100):
            if os.path.exists(self.sock):
                return
            time.sleep(0.1)
        raise RuntimeError("broker socket did not appear")

    def stop_broker(self):
        self.proc.terminate()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()

    def broker_log(self):
        self.log.flush()
        return open(self.log_path).read()


def scenario(sess, name, burst):
    """Connect, send `burst` frames back to back, then watch."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(sess.sock)
    for _ in range(6):
        tc.recv_exact(s, tc.PKT.size)
    bufs = [mkbuf(i) for i in range(3)]
    ids = [os.fstat(b).st_ino for b in bufs]
    ev = []
    t0 = time.monotonic()

    def pump(dur):
        end = time.monotonic() + dur
        s.settimeout(0.05)
        while time.monotonic() < end:
            try:
                pk = tc.PKT.unpack(tc.recv_exact(s, tc.PKT.size))
            except (socket.timeout, OSError):
                continue
            if pk[0] == 4:      # EV_RELEASE
                bid = (pk[5] & 0xffffffff) | (pk[6] << 32)
                ev.append((time.monotonic() - t0, ids.index(bid) if bid in ids else -1))

    pump(0.4)
    check = tc.check
    sent = time.monotonic() - t0
    for i in range(burst):
        tx.attach_commit(s, bufs[i], W, H, STRIDE, 0, shm=False, modifier=0, seq=1 + i)
    pump(1.2)
    rel = [e[1] for e in ev]
    first_second = [e[1] for e in ev if e[0] - sent < 1.0]
    if burst == 3:
        check(sorted(first_second) == [0, 1],
              "%s: the first two frames of the burst are released within a second (got %r)" % (name, first_second))
        check(2 not in rel,
              "%s: the newest frame, now on screen, is NOT released while it is shown" % name)
    else:
        check(rel == [], "%s: a single frame is shown, not released (got %r)" % (name, rel))
    log = sess.broker_log()
    check("is shown now" in log,
          "%s: the frame kept by the probe window was imported and shown" % name)
    # a later frame replaces it: exactly that release follows
    before = len(ev)
    tx.attach_commit(s, bufs[(burst) % 3], W, H, STRIDE, 0, shm=False, modifier=0, seq=20)
    pump(1.0)
    got = [e[1] for e in ev[before:]]
    check(got == [burst - 1],
          "%s: the replaced frame is released when its replacement is shown (got %r want %r)" % (name, got, [burst - 1]))
    for b in bufs:
        os.close(b)
    s.close()
    time.sleep(0.3)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: test_probe.py /path/to/broker")
    broker = os.path.abspath(sys.argv[1])
    for tool in ("mutter", "dbus-run-session"):
        if not shutil.which(tool):
            skip(tool + " is not installed")
    if not os.access("/dev/udmabuf", os.R_OK | os.W_OK):
        skip("/dev/udmabuf is not usable")
    with tempfile.TemporaryDirectory(prefix="nbprobe", ignore_cleanup_errors=True) as tmp:
        sess = Session(broker, tmp, "a")
        if not sess.start_compositor():
            sess.stop_compositor()
            skip("mutter --headless did not come up")
        try:
            sess.start_broker()
            time.sleep(0.5)
            if "LINEAR" not in sess.broker_log():
                skip("this compositor does not advertise a LINEAR dma-buf pair")
            scenario(sess, "burst of 3", 3)
            sess.stop_broker()
            sess.start_broker()
            scenario(sess, "single replay frame", 1)
        finally:
            try:
                sess.stop_broker()
            except Exception:
                pass
            sess.stop_compositor()
    if tc.FAILS:
        print("%d probe check(s) FAILED" % len(tc.FAILS))
        sys.exit(1)
    print("probe tests passed")


if __name__ == "__main__":
    main()
