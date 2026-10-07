#!/usr/bin/env python3
"""vnc_watch.py HOST:PORT OUTDIR X,Y,W,H [seconds] — a timeline of what a cursor-capable VNC client
is shown (the v3-maxfps box stage, V3_DISPLAY.md §8.13: "never two cursors"). Python 3 stdlib only.
Asks for Raw pixels, the Cursor With Alpha pseudo-encoding (-314), the rich cursor (-239) and
DesktopSize (-223); requests an incremental update after every update. Writes OUTDIR/timeline.txt:
  <t_wall> FB rects=<n> roi=<fnv of the ROI's pixels>         (only when the ROI's pixels changed)
  <t_wall> CURSOR enc=<e> w=<w> h=<h> hot=<x>,<y> visible_px=<n> fnv=<fnv of the words as received>
  <t_wall> DESKTOP w=<w> h=<h>
and OUTDIR/roi_<fnv>.ppm for each distinct ROI picture. Stops after `seconds` (default 600) or when
OUTDIR/stop exists. The ROI is in the console's pixels."""
import os, socket, struct, sys, time

def fnv(b):
    h = 0xCBF29CE484222325
    for x in b:
        h = ((h ^ x) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h

def rx(s, n):
    b = bytearray()
    while len(b) < n:
        c = s.recv(min(n - len(b), 1 << 20))
        if not c:
            raise EOFError("closed")
        b += c
    return bytes(b)

def main():
    host, port = sys.argv[1].rsplit(":", 1)
    out = sys.argv[2]; os.makedirs(out, exist_ok=True)
    rx0, ry0, rw, rh = (int(v) for v in sys.argv[3].split(","))
    secs = float(sys.argv[4]) if len(sys.argv) > 4 else 600
    tl = open(os.path.join(out, "timeline.txt"), "a", buffering=1)
    def log(m):
        tl.write("%.3f %s\n" % (time.time(), m))
    end = time.time() + secs
    s = None
    while time.time() < end:
        try:
            s = socket.create_connection((host, int(port)), timeout=5); break
        except OSError:
            time.sleep(0.5)
    if s is None:
        log("NOCONNECT"); return
    s.settimeout(None)
    ver = rx(s, 12); s.sendall(b"RFB 003.008\n")
    n = rx(s, 1)[0]; types = rx(s, n)
    if 1 not in types:
        log("NO_NONE_SECURITY %r" % types); return
    s.sendall(b"\x01")
    if struct.unpack(">I", rx(s, 4))[0] != 0:
        log("SECURITY_FAIL"); return
    s.sendall(b"\x01")
    w, h = struct.unpack(">HH", rx(s, 4)); rx(s, 16); nl = struct.unpack(">I", rx(s, 4))[0]; name = rx(s, nl)
    log("CONNECTED %s w=%d h=%d name=%s" % (ver.strip().decode(), w, h, name.decode(errors="replace")))
    pf = struct.pack(">BBBBHHHBBB3x", 32, 24, 0, 1, 255, 255, 255, 16, 8, 0)
    s.sendall(b"\x00\x00\x00\x00" + pf)
    encs = [0, -314, -239, -223]
    s.sendall(struct.pack(">BxH", 2, len(encs)) + b"".join(struct.pack(">i", e) for e in encs))
    fb = {}  # only the ROI's rows: (y) -> bytearray(rw*4)
    def roi_bytes():
        return b"".join(bytes(fb.get(y, bytes(rw * 4))) for y in range(ry0, ry0 + rh))
    last = None
    s.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, w, h))
    while time.time() < end and not os.path.exists(os.path.join(out, "stop")):
        t = rx(s, 1)[0]
        if t == 0:
            rx(s, 1); nr = struct.unpack(">H", rx(s, 2))[0]; touched = False
            for _ in range(nr):
                x, y, rw_, rh_, e = struct.unpack(">HHHHi", rx(s, 12))
                if e == 0:
                    data = rx(s, rw_ * rh_ * 4)
                    for row in range(rh_):
                        yy = y + row
                        if ry0 <= yy < ry0 + rh and x < rx0 + rw and x + rw_ > rx0:
                            a, b_ = max(x, rx0), min(x + rw_, rx0 + rw)
                            line = fb.setdefault(yy, bytearray(rw * 4))
                            src = data[(row * rw_ + (a - x)) * 4:(row * rw_ + (b_ - x)) * 4]
                            line[(a - rx0) * 4:(b_ - rx0) * 4] = src; touched = True
                elif e == -314:
                    enc = struct.unpack(">i", rx(s, 4))[0]
                    px = rx(s, rw_ * rh_ * 4) if enc == 0 else b""
                    words = [struct.unpack("<I", px[i:i + 4])[0] for i in range(0, len(px), 4)] if px else []
                    vis = sum(1 for p in words if p >> 24)
                    log("CURSOR enc=-314 w=%d h=%d hot=%d,%d visible_px=%d fnv=%#018x" % (rw_, rh_, x, y, vis, fnv(px)))
                elif e == -239:
                    px = rx(s, rw_ * rh_ * 4); mask = rx(s, ((rw_ + 7) // 8) * rh_)
                    vis = sum(bin(b).count("1") for b in mask)
                    log("CURSOR enc=-239 w=%d h=%d hot=%d,%d visible_px=%d fnv=%#018x" % (rw_, rh_, x, y, vis, fnv(px + mask)))
                elif e == -223:
                    w, h = rw_, rh_; fb.clear(); log("DESKTOP w=%d h=%d" % (w, h))
                    s.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, w, h))
                else:
                    log("UNKNOWN_ENC %d" % e); return
            if touched:
                rb = roi_bytes(); hsh = fnv(rb)
                if hsh != last:
                    last = hsh; log("FB rects=%d roi=%#018x" % (nr, hsh))
                    p = os.path.join(out, "roi_%016x.ppm" % hsh)
                    if not os.path.exists(p):
                        with open(p, "wb") as f:
                            f.write(b"P6\n%d %d\n255\n" % (rw, rh))
                            f.write(bytes(v for i in range(0, len(rb), 4) for v in (rb[i + 2], rb[i + 1], rb[i])))
            s.sendall(struct.pack(">BBHHHH", 3, 1, 0, 0, w, h))
        elif t == 1:
            rx(s, 1); first, nc = struct.unpack(">HH", rx(s, 4)); rx(s, nc * 6)
        elif t == 2:
            pass
        elif t == 3:
            rx(s, 3); rx(s, struct.unpack(">I", rx(s, 4))[0])
        else:
            log("UNKNOWN_MSG %d" % t); return
    log("END")

if __name__ == "__main__":
    main()
