#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""vnc_cursor.py — the cursor QEMU's VNC server hands a cursor-capable client, for the broker lane
(docs/design/V3_DISPLAY.md §8.13: in hover the console gets the guest cursor through QEMU's cursor
API, and a VNC client draws it as a real pointer). Python 3 stdlib only.

  vnc_cursor.py HOST:PORT [out.pam]   one line: VNC_CURSOR w= h= hot= visible_px= bbox_rel_hot=
                                      fnv_rel_hot= (the format of xcursor.py's CURSOR line, the same
                                      FNV over the visible pixels' positions relative to the hot spot
                                      and their PREMULTIPLIED ARGB — what XFixes reports — so the two
                                      lines compare directly); with out.pam, the image as a PAM that
                                      `xcursor.py compare` reads. `VNC_CURSOR none` when the server sent
                                      no cursor within the deadline (no cursor defined on the console).
  vnc_cursor.py --selftest            a fake server speaking QEMU 10.2.4's bytes (ui/vnc.c:992-1027)

The client asks for the VMware alpha-cursor pseudo-encoding (-314, QEMU's VNC_ENCODING_ALPHA_CURSOR)
and the rich cursor (-239) as a fallback; QEMU sends the console's cursor right after SetEncodings
(ui/vnc.c:2237) and again on every define. Alpha cursor: rect x,y = the hot spot, w,h, encoding -314,
then s32 0 (raw) and w*h QEMUCursor words written as they are — host-endian 0xAARRGGBB with
straight alpha (what kf3 defines) — so each colour is premultiplied here before the FNV.
"""
import socket
import struct
import sys
import threading
import time

ALPHA_CURSOR = -314
RICH_CURSOR = -239
DESKTOP_SIZE = -223


def recv_exact(s, n):
    b = b""
    while len(b) < n:
        c = s.recv(n - len(b))
        if not c:
            raise EOFError("the server closed the connection")
        b += c
    return b


def fnv(data):
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def premultiply(px):
    """0xAARRGGBB straight -> premultiplied, rounded."""
    a = (px >> 24) & 0xFF
    out = a << 24
    for sh in (16, 8, 0):
        out |= ((((px >> sh) & 0xFF) * a + 127) // 255) << sh
    return out


def summarize(w, h, hot, px):
    """xcursor.py's summary over premultiplied ARGB words."""
    vis = [(i % w, i // w, p) for i, p in enumerate(px) if (p >> 24) & 0xFF]
    if not vis:
        return "visible_px=0 bbox_rel_hot=none fnv_rel_hot=none"
    xs = [v[0] - hot[0] for v in vis]
    ys = [v[1] - hot[1] for v in vis]
    blob = b""
    for x, y, p in vis:
        blob += (x - hot[0]).to_bytes(2, "little", signed=True)
        blob += (y - hot[1]).to_bytes(2, "little", signed=True)
        blob += (p & 0xFFFFFFFF).to_bytes(4, "little")
    return "visible_px=%d bbox_rel_hot=%d,%d,%d,%d fnv_rel_hot=%#018x" % (
        len(vis), min(xs), min(ys), max(xs), max(ys), fnv(blob))


def write_pam(path, w, h, hot, px):
    with open(path, "wb") as o:
        o.write(("P7\n# hot %d %d\nWIDTH %d\nHEIGHT %d\nDEPTH 4\nMAXVAL 255\n"
                 "TUPLTYPE RGB_ALPHA\nENDHDR\n" % (hot[0], hot[1], w, h)).encode())
        for p in px:
            o.write(bytes([(p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF, (p >> 24) & 0xFF]))


def grab(host, port, deadline=8.0):
    """The first cursor the server sends: (w, h, hot, premultiplied ARGB words) or None."""
    s = socket.create_connection((host, port), timeout=deadline)
    s.settimeout(deadline)
    ver = recv_exact(s, 12)
    if not ver.startswith(b"RFB "):
        raise ValueError("not an RFB server: %r" % ver)
    s.sendall(b"RFB 003.008\n")
    n = recv_exact(s, 1)[0]
    if n == 0:
        raise ValueError("the server refused: %r" % recv_exact(s, struct.unpack(">I", recv_exact(s, 4))[0]))
    types = recv_exact(s, n)
    if 1 not in types:
        raise ValueError("no 'None' security type offered: %r" % list(types))
    s.sendall(b"\x01")
    if struct.unpack(">I", recv_exact(s, 4))[0] != 0:
        raise ValueError("security handshake failed")
    s.sendall(b"\x01")  # ClientInit: shared
    fw, fh = struct.unpack(">HH", recv_exact(s, 4))
    recv_exact(s, 16)  # the server's pixel format
    recv_exact(s, struct.unpack(">I", recv_exact(s, 4))[0])  # the desktop name
    # SetPixelFormat: 32 bpp, depth 24, little-endian, true colour, BGRX (rich-cursor pixels decode)
    s.sendall(struct.pack(">BxxxBBBBHHHBBBxxx", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
    encs = [0, ALPHA_CURSOR, RICH_CURSOR, DESKTOP_SIZE]
    s.sendall(struct.pack(">BxH", 2, len(encs)) + b"".join(struct.pack(">i", e) for e in encs))
    s.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, fw, fh))
    t0 = time.time()
    while time.time() - t0 < deadline:
        t = recv_exact(s, 1)[0]
        if t == 0:  # FramebufferUpdate
            recv_exact(s, 1)
            (nrects,) = struct.unpack(">H", recv_exact(s, 2))
            for _ in range(nrects):
                x, y, w, h, enc = struct.unpack(">HHHHi", recv_exact(s, 12))
                if enc == ALPHA_CURSOR:
                    (inner,) = struct.unpack(">i", recv_exact(s, 4))
                    if inner != 0:
                        raise ValueError("alpha cursor data in encoding %d, not raw" % inner)
                    raw = recv_exact(s, w * h * 4)
                    px = [premultiply(struct.unpack("<I", raw[i:i + 4])[0]) for i in range(0, len(raw), 4)]
                    s.close()
                    return w, h, (x, y), px
                if enc == RICH_CURSOR:
                    raw = recv_exact(s, w * h * 4)
                    mask = recv_exact(s, ((w + 7) // 8) * h)
                    px = []
                    for i in range(w * h):
                        on = (mask[(i // w) * ((w + 7) // 8) + (i % w) // 8] >> (7 - i % 8)) & 1
                        (v,) = struct.unpack("<I", raw[4 * i:4 * i + 4])
                        px.append(((0xFF << 24) | (v & 0xFFFFFF)) if on else 0)
                    s.close()
                    return w, h, (x, y), px
                if enc == 0:
                    recv_exact(s, w * h * 4)
                elif enc == DESKTOP_SIZE:
                    fw, fh = w, h
                else:
                    raise ValueError("an encoding this client did not ask for: %d" % enc)
            s.sendall(struct.pack(">BBHHHH", 3, 1, 0, 0, fw, fh))
        elif t == 1:  # SetColourMapEntries
            recv_exact(s, 3)
            recv_exact(s, 6 * struct.unpack(">H", recv_exact(s, 2))[0])
        elif t == 2:  # Bell
            pass
        elif t == 3:  # ServerCutText
            recv_exact(s, 3)
            recv_exact(s, struct.unpack(">I", recv_exact(s, 4))[0])
        else:
            raise ValueError("an unknown server message %d" % t)
    s.close()
    return None


def selftest():
    """A fake QEMU: ServerInit, then (after SetEncodings) one 2x1 alpha cursor, hot 1,0, written as
    vnc_cursor_define does — QEMUCursor words, little-endian, straight alpha."""
    words = [0x80FF0000, 0x00000000]  # half-covered red, transparent
    srv = socket.socket()
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)
    port = srv.getsockname()[1]

    def serve():
        c, _ = srv.accept()
        c.sendall(b"RFB 003.008\n")
        recv_exact(c, 12)
        c.sendall(b"\x01\x01")
        recv_exact(c, 1)
        c.sendall(struct.pack(">I", 0))
        recv_exact(c, 1)
        c.sendall(struct.pack(">HH", 64, 48) + bytes(16) + struct.pack(">I", 3) + b"kf0")
        recv_exact(c, 20)  # SetPixelFormat
        n = struct.unpack(">BxH", recv_exact(c, 4))[1]
        recv_exact(c, 4 * n)
        c.sendall(struct.pack(">BxH", 0, 1) + struct.pack(">HHHHi", 1, 0, 2, 1, ALPHA_CURSOR)
                  + struct.pack(">i", 0) + b"".join(struct.pack("<I", w) for w in words))
        time.sleep(0.5)
        c.close()

    th = threading.Thread(target=serve, daemon=True)
    th.start()
    got = grab("127.0.0.1", port, 5)
    th.join(2)
    srv.close()
    assert got is not None, "no cursor"
    w, h, hot, px = got
    assert (w, h, hot) == (2, 1, (1, 0)), (w, h, hot)
    assert px == [0x80800000, 0], ["%#x" % p for p in px]  # 0xff * 0x80 / 255 -> 0x80
    print("VNC_CURSOR_SELFTEST ok %dx%d hot=%d,%d %s" % (w, h, hot[0], hot[1], summarize(w, h, hot, px)))


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--selftest":
        selftest()
        return
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    host, port = sys.argv[1].rsplit(":", 1)
    got = grab(host, int(port))
    if got is None:
        print("VNC_CURSOR none")
        return
    w, h, hot, px = got
    print("VNC_CURSOR w=%d h=%d hot=%d,%d %s" % (w, h, hot[0], hot[1], summarize(w, h, hot, px)))
    if len(sys.argv) > 2:
        write_pam(sys.argv[2], w, h, hot, px)


if __name__ == "__main__":
    main()
