#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""dri3_refusal.py — ask the host X server to import malformed GPU buffers through the display
broker, and record what the broker says on the wire (docs/design/V3_DISPLAY.md §8.15 rows 1-3:
nvkvm-pv `badf2d7` sends an unsolicited EV_FORMAT x=0 for a refused import — on X11 one for each
alpha twin, XR24 and AR24 — and forgets it when the connection ends). Python 3 stdlib only.

  dri3_refusal.py --broker SOCK --source SRC [--udmabuf] [--variants a,b,...]
      SRC is nvkvm-pv's `nvkvm-broker-dmabuf-src --serve SRC`: a real dma-buf allocated on the
      host GPU, handed over with its descriptor ({w, h, stride, offset, fourcc, modifier, size}).
      Every variant runs on its OWN connection: wait for the first FRAME; QUERY_FORMAT (XR24, the
      declared modifier) — never AR24; ATTACH the buffer under the variant's descriptor; COMMIT;
      read for 2 s; QUERY_FORMAT again; disconnect. One line each:
        DRI3_VARIANT <name> decl=WxH/stride/offset/modifier query=<x> after=[fourcc:x ...]
                     frames=N releases=N requery=<x>
      `after` lists every EV_FORMAT the broker sent after the ATTACH, asked or not. A DRI3
      refusal on X11 reads `after=[XR24:0 AR24:0]` and `requery=0`; the next variant's `query=1`
      on a NEW connection is the refusal not outliving its connection.
  dri3_refusal.py --selftest
      a fake broker and a fake source speaking the same bytes, one refusal scripted (no GPU)

The broker accepts root or its own user. Variants (all keep the buffer's real fd; each changes
ONE descriptor field the broker's validator still passes — `4w <= stride <= 8w + 4096`,
`stride * h + offset <= size` — so whatever refuses it is the X server's import):
  own          the buffer's own geometry: the control, must be shown (frames > 0, no x=0)
  pitch+4      stride + 4 bytes: not a multiple of a block-linear GOB's 64-byte width
  pitch+64     stride + 64: GOB-aligned but not the pitch the buffer was allocated with
  offset+4     plane 0 at byte 4
  kind         the declared modifier's other advertised family (0x606xxx <-> 0xe08xxx: page kind
               0x06 <-> 0x08, compression 0 <-> 1); the buffer was allocated with the first
  bh0          the declared block height changed to one GOB (advertised; allocated with another)
  udmabuf      (--udmabuf) a host-memory udmabuf of the same size declared with the buffer's
               block-linear modifier: a dma-buf the NVIDIA driver did not export
  tail4k       plane 0 at byte 4096 and as many rows as then fit: the broker's linear extent
               `stride * h + offset` is the whole buffer, the block-linear surface (rows rounded
               up to whole blocks) ends past it
  tail64k      the same at byte 65536
  udmabuf_short (--udmabuf) a udmabuf of `stride * (h - 12)` bytes declared with h - 12 rows: its
               linear extent fits, its block-linear one (whole blocks of rows) does not
  own_again    the control again, on a new connection after all of the above
(Added 2026-10-04 after run brkF2: the NVIDIA DDX 580.159.04 imported every one of the first seven
without an X error. The `tail` variants ask whether it checks a block-linear extent against the
dma-buf's size at all; a GPU read past a udmabuf's end, if it happens, is a fault in the X server's
context — check the host's dmesg for an Xid after the run.)
"""
import argparse
import fcntl
import os
import socket
import struct
import sys
import threading
import time

PKT = struct.Struct("<HHIiiII")  # nvkvm_broker_pkt, 24 bytes
CMD = struct.Struct("<HHIIIIIQII")  # nvkvm_broker_cmd, 40 bytes
DESC = struct.Struct("<5I4xQQ")  # dmabuf_source.c struct src_desc, 40 bytes
EV_HELLO, EV_FRAME, EV_RELEASE, EV_FORMAT = 1, 3, 4, 16
CMD_ATTACH, CMD_COMMIT, CMD_QUERY = 1, 2, 6
XR24, AR24 = 0x34325258, 0x34325241
NAMES = {XR24: "XR24", AR24: "AR24"}
assert PKT.size == 24 and CMD.size == 40 and DESC.size == 40


def fcc(v):
    return NAMES.get(v, "%#x" % v)


def get_source(path):
    """(descriptor dict, fd) from dmabuf_source --serve."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(10)
    s.connect(path)
    msg, fds, _, _ = socket.recv_fds(s, DESC.size, 1)
    s.close()
    if len(msg) != DESC.size or len(fds) != 1:
        raise RuntimeError("the source sent %d bytes and %d fds" % (len(msg), len(fds)))
    w, h, stride, off, fourcc, mod, size = DESC.unpack(msg)
    return dict(w=w, h=h, stride=stride, offset=off, fourcc=fourcc, modifier=mod, size=size), fds[0]


def make_udmabuf(size):
    """A sealed memfd turned into a dma-buf by /dev/udmabuf (UDMABUF_CREATE)."""
    page = os.sysconf("SC_PAGE_SIZE")
    size = (size + page - 1) // page * page
    mfd = os.memfd_create("kf-dri3-udmabuf", os.MFD_ALLOW_SEALING)
    os.ftruncate(mfd, size)
    fcntl.fcntl(mfd, 1033, 2)  # F_ADD_SEALS, F_SEAL_SHRINK
    dev = os.open("/dev/udmabuf", os.O_RDWR)
    try:
        # _IOW('u', 0x42, struct udmabuf_create {u32 memfd, flags; u64 offset, size})
        # ⊘ [box 54032077, run brkF1, 2026-10-04] an immutable bytes argument makes fcntl.ioctl
        # return the buffer, not the call's result (the new fd): a mutable one returns the int
        fd = fcntl.ioctl(dev, 0x40187542, bytearray(struct.pack("<IIQQ", mfd, 1, 0, size)), True)
    finally:
        os.close(dev)
        os.close(mfd)
    if not isinstance(fd, int) or fd < 0:
        raise RuntimeError("UDMABUF_CREATE returned %r" % (fd,))
    return fd


class Conn:
    def __init__(self, path):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(5)
        self.s.connect(path)
        self.buf = b""
        self.caps = None

    def send(self, typ, w=0, h=0, stride=0, off=0, fourcc=0, mod=0, fd=None):
        b = CMD.pack(typ, 0, w, h, stride, off, fourcc, mod, 0, 0)
        if fd is None:
            self.s.sendall(b)
        else:
            socket.send_fds(self.s, [b], [fd])

    def packets(self, until):
        """Every packet until `until` (a monotonic deadline)."""
        out = []
        while True:
            left = until - time.monotonic()
            if left <= 0:
                return out
            self.s.settimeout(left)
            try:
                c = self.s.recv(4096)
            except socket.timeout:
                return out
            if not c:
                out.append(None)
                return out
            self.buf += c
            while len(self.buf) >= PKT.size:
                out.append(PKT.unpack(self.buf[:PKT.size]))
                self.buf = self.buf[PKT.size:]

    def first_frame(self):
        end = time.monotonic() + 8
        while time.monotonic() < end:
            for p in self.packets(min(end, time.monotonic() + 0.5)):
                if p is None:
                    return False
                if p[0] == EV_HELLO:
                    self.caps = p[6]
                if p[0] == EV_FRAME:
                    return True
        return False

    def close(self):
        self.s.close()


def run_variant(broker, name, d, fd, settle=2.0):
    c = Conn(broker)
    try:
        if not c.first_frame():
            return "DRI3_VARIANT %s no FRAME from the broker within 8 s (another client attached?)" % name
        c.send(CMD_QUERY, fourcc=XR24, mod=d["modifier"])
        q = None
        for p in c.packets(time.monotonic() + 2):
            if p and p[0] == EV_FORMAT and p[4] == XR24:
                q = p[3]
                break
        c.send(CMD_ATTACH, d["w"], d["h"], d["stride"], d["offset"], XR24, d["modifier"], fd)
        c.send(CMD_COMMIT)
        after, frames, releases, closed = [], 0, 0, False
        for p in c.packets(time.monotonic() + settle):
            if p is None:
                closed = True
                continue
            if p[0] == EV_FORMAT:
                after.append("%s:%d%s" % (fcc(p[4]), p[3],
                                          "" if (p[5] | p[6] << 32) == d["modifier"]
                                          else "@%#x" % (p[5] | p[6] << 32)))
            elif p[0] == EV_FRAME:
                frames += 1
            elif p[0] == EV_RELEASE:
                releases += 1
        rq = None
        if not closed:
            c.send(CMD_QUERY, fourcc=XR24, mod=d["modifier"])
            for p in c.packets(time.monotonic() + 2):
                if p and p[0] == EV_FORMAT and p[4] == XR24:
                    rq = p[3]
                    break
        return ("DRI3_VARIANT %s decl=%dx%d/%d/%d/%#018x query=%s after=[%s] frames=%d releases=%d "
                "requery=%s%s" % (name, d["w"], d["h"], d["stride"], d["offset"], d["modifier"],
                                  q, " ".join(after), frames, releases, rq,
                                  " BROKER_CLOSED" if closed else ""))
    finally:
        c.close()


def variants(src, want, ufd):
    w, h, st, off, mod = src["w"], src["h"], src["stride"], src["offset"], src["modifier"]
    hh = max(1, h - 32)  # room for a wider pitch or an offset inside the real extent
    # the other advertised block-linear family: page kind 0x06 <-> 0x08 with compression 0 <-> 1
    # (DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D: k = bits 12-19, c = bits 23-25)
    fam = 0xE08000 if (mod & 0xFFF000) == 0x606000 else 0x606000
    kind = (mod & ~0xFFF000) | fam
    table = [
        ("own", dict(w=w, h=h, stride=st, offset=off, modifier=mod), None),
        ("pitch+4", dict(w=w, h=hh, stride=st + 4, offset=off, modifier=mod), None),
        ("pitch+64", dict(w=w, h=hh, stride=st + 64, offset=off, modifier=mod), None),
        ("offset+4", dict(w=w, h=hh, stride=st, offset=off + 4, modifier=mod), None),
        ("kind", dict(w=w, h=h, stride=st, offset=off, modifier=kind), None),
        ("bh0", dict(w=w, h=h, stride=st, offset=off, modifier=mod & ~0xF), None),
        ("udmabuf", dict(w=w, h=h, stride=st, offset=0, modifier=mod), "udmabuf"),
        ("tail4k", dict(w=w, h=(src["size"] - off - 4096) // st, stride=st, offset=off + 4096,
                        modifier=mod), None),
        ("tail64k", dict(w=w, h=(src["size"] - off - 65536) // st, stride=st, offset=off + 65536,
                         modifier=mod), None),
        ("udmabuf_short", dict(w=w, h=max(1, h - 12), stride=st, offset=0, modifier=mod),
         "udmabuf_short"),
        ("own_again", dict(w=w, h=h, stride=st, offset=off, modifier=mod), None),
    ]
    for name, d, which in table:
        if want and name not in want:
            continue
        if which and (ufd is None or ufd.get(which) is None):
            continue
        yield name, d, which


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--broker")
    ap.add_argument("--source")
    ap.add_argument("--udmabuf", action="store_true")
    ap.add_argument("--variants", default="")
    ap.add_argument("--settle", type=float, default=2.0)
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args(argv)
    if a.selftest:
        return selftest()
    if not a.broker or not a.source:
        ap.error("--broker and --source are required")
    src, fd = get_source(a.source)
    print("DRI3_SOURCE %dx%d stride=%d offset=%d fourcc=%s modifier=%#018x size=%d" % (
        src["w"], src["h"], src["stride"], src["offset"], fcc(src["fourcc"]), src["modifier"],
        src["size"]), flush=True)
    ufd = {}
    if a.udmabuf:
        for which, size in (("udmabuf", src["size"]),
                            ("udmabuf_short", src["stride"] * max(1, src["h"] - 12))):
            try:
                ufd[which] = make_udmabuf(size)
                print("DRI3_UDMABUF %s %d bytes" % (which, os.fstat(ufd[which]).st_size or
                                                     os.lseek(ufd[which], 0, os.SEEK_END)),
                      flush=True)
            except (OSError, RuntimeError) as e:
                print("DRI3_UDMABUF %s unavailable: %s" % (which, e), flush=True)
    want = set(x for x in a.variants.split(",") if x)
    for name, d, which in variants(src, want, ufd):
        try:
            print(run_variant(a.broker, name, d, ufd[which] if which else fd, a.settle),
                  flush=True)
        except OSError as e:
            print("DRI3_VARIANT %s ERROR %s" % (name, e), flush=True)
        time.sleep(0.5)
    print("DRI3_DONE", flush=True)
    return 0


def selftest():
    """A fake broker (refuses stride != the source's, on X11's terms) and a fake source."""
    import tempfile
    d = tempfile.mkdtemp()
    bsock, ssock = os.path.join(d, "b.sock"), os.path.join(d, "s.sock")
    mfd = os.memfd_create("selftest")
    os.ftruncate(mfd, 1 << 20)
    src = (512, 512, 2048, 0, XR24, 0x0300000000606014, 1 << 20)
    ls = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    ls.bind(ssock)
    ls.listen(8)
    lb = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    lb.bind(bsock)
    lb.listen(8)
    stop = threading.Event()

    def serve_source():
        while not stop.is_set():
            try:
                c, _ = ls.accept()
            except OSError:
                return
            socket.send_fds(c, [DESC.pack(*src)], [mfd])
            c.close()

    def pkt(seq, typ, x=0, y=0, w0=0, w1=0):
        return PKT.pack(typ, 0, seq, x, y, w0, w1)

    def serve_broker():
        while not stop.is_set():
            try:
                c, _ = lb.accept()
            except OSError:
                return
            refused, seq = set(), 0
            c.sendall(pkt(1, EV_HELLO, 0, 0, 2, 0xFFF) + pkt(2, EV_FRAME))
            seq = 2
            while True:
                try:
                    msg, fds, _, _ = socket.recv_fds(c, CMD.size, 1)
                except OSError:
                    break
                if not msg:
                    break
                for f in fds:
                    os.close(f)
                t, _, w, h, st, off, fourcc, mod, _, _ = CMD.unpack(msg)
                if t == CMD_QUERY:
                    seq += 1
                    c.sendall(pkt(seq, EV_FORMAT, 0 if (fourcc, mod) in refused else 1,
                                  fourcc, mod & 0xFFFFFFFF, mod >> 32))
                elif t == CMD_ATTACH and st != src[2]:
                    for f in (XR24, AR24):
                        refused.add((f, mod))
                        seq += 1
                        c.sendall(pkt(seq, EV_FORMAT, 0, f, mod & 0xFFFFFFFF, mod >> 32))
                elif t == CMD_COMMIT:
                    seq += 1
                    c.sendall(pkt(seq, EV_FRAME))
            c.close()

    threading.Thread(target=serve_source, daemon=True).start()
    threading.Thread(target=serve_broker, daemon=True).start()
    s, fd = get_source(ssock)
    lines = [run_variant(bsock, n, dd, fd, 0.3) for n, dd, _ in
             variants(s, {"own", "pitch+4", "own_again"}, {})]
    stop.set()
    ls.close()
    lb.close()
    for line in lines:
        print(line)
    ok = ("after=[] frames=1" in lines[0] and "query=1" in lines[0]
          and "after=[XR24:0 AR24:0]" in lines[1] and "requery=0" in lines[1]
          and "query=1" in lines[2] and "after=[]" in lines[2])
    print("SELFTEST %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
