#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""xcursor.py — the X server's CURRENT cursor, through XFixes, for the broker lane
(docs/design/V3_DISPLAY.md §8.12, the hover-mode host cursor). Runs on the host AND in the guest:
it needs only libX11.so.6 and libXfixes.so.3 (ctypes), and DISPLAY/XAUTHORITY from the environment.

  xcursor.py image [out.pam]   one line: CURSOR w= h= hot= visible_px= bbox_rel_hot= fnv_rel_hot=
                               (the FNV covers the alpha>0 pixels' positions RELATIVE TO THE HOT SPOT
                               and their ARGB, so the same image in a larger cursor buffer — the
                               guest's 64x64 plane against the X theme's 24x24 — digests the same);
                               with out.pam, the image as a PAM (RGB_ALPHA, straight from XFixes)
  xcursor.py hide <seconds>    XFixesHideCursor on the root window for <seconds>: the X server hides
                               the cursor while this client lives (the guest "hides its cursor")
  xcursor.py pointer           POINTER x y — the pointer's position on the root window
  xcursor.py compare a.pam b.pam   CURSOR_COMPARE: the two images aligned at their hot spots
                               (PAM comment lines carry the hot spot): max channel difference over
                               the union of their visible pixels, and the visible-pixel counts
"""
import ctypes
import sys
import time


class XFixesCursorImage(ctypes.Structure):
    _fields_ = [
        ("x", ctypes.c_short),
        ("y", ctypes.c_short),
        ("width", ctypes.c_ushort),
        ("height", ctypes.c_ushort),
        ("xhot", ctypes.c_ushort),
        ("yhot", ctypes.c_ushort),
        ("cursor_serial", ctypes.c_ulong),
        ("pixels", ctypes.POINTER(ctypes.c_ulong)),
        ("atom", ctypes.c_ulong),
        ("name", ctypes.c_char_p),
    ]


def libs():
    x = ctypes.CDLL("libX11.so.6")
    f = ctypes.CDLL("libXfixes.so.3")
    x.XOpenDisplay.restype = ctypes.c_void_p
    x.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x.XDefaultRootWindow.restype = ctypes.c_ulong
    x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
    x.XFlush.argtypes = [ctypes.c_void_p]
    x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    x.XFree.argtypes = [ctypes.c_void_p]
    x.XCloseDisplay.argtypes = [ctypes.c_void_p]
    f.XFixesGetCursorImage.restype = ctypes.POINTER(XFixesCursorImage)
    f.XFixesGetCursorImage.argtypes = [ctypes.c_void_p]
    f.XFixesHideCursor.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    f.XFixesShowCursor.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    x.XQueryPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong] + [ctypes.c_void_p] * 7
    f.XFixesQueryVersion.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    d = x.XOpenDisplay(None)
    if not d:
        print("CURSOR_ERROR cannot open the display (DISPLAY/XAUTHORITY?)")
        sys.exit(2)
    # ⊘ [box 54032077, run brkA, 2026-10-03] without the version handshake the guest's X server
    # answered XFixesGetCursorImage with nothing (Xvfb did not mind): XFixes requires it first
    maj, mnr = ctypes.c_int(6), ctypes.c_int(0)
    f.XFixesQueryVersion(d, ctypes.byref(maj), ctypes.byref(mnr))
    return x, f, d


def fnv(data):
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def summarize(w, h, hot, px):
    """px: w*h 32-bit ARGB words."""
    vis = [(i % w, i // w, p) for i, p in enumerate(px) if (p >> 24) & 0xFF]
    if not vis:
        return "visible_px=0 bbox_rel_hot=none fnv_rel_hot=none"
    xs = [v[0] - hot[0] for v in vis]
    ys = [v[1] - hot[1] for v in vis]
    blob = bytearray()
    for (x, y, p) in vis:
        blob += (x - hot[0]).to_bytes(2, "little", signed=True)
        blob += (y - hot[1]).to_bytes(2, "little", signed=True)
        blob += p.to_bytes(4, "little")
    return "visible_px=%d bbox_rel_hot=%d,%d,%d,%d fnv_rel_hot=%#018x" % (
        len(vis), min(xs), min(ys), max(xs), max(ys), fnv(blob))


def write_pam(path, w, h, hot, px):
    with open(path, "wb") as o:
        o.write(("P7\n# hot %d %d\nWIDTH %d\nHEIGHT %d\nDEPTH 4\nMAXVAL 255\n"
                 "TUPLTYPE RGB_ALPHA\nENDHDR\n" % (hot[0], hot[1], w, h)).encode())
        for p in px:
            o.write(bytes([(p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF, (p >> 24) & 0xFF]))


def read_pam(path):
    raw = open(path, "rb").read()
    head, _, body = raw.partition(b"ENDHDR\n")
    hot, w, h = (0, 0), 0, 0
    for line in head.decode().splitlines():
        t = line.split()
        if line.startswith("# hot"):
            hot = (int(t[2]), int(t[3]))
        elif t and t[0] == "WIDTH":
            w = int(t[1])
        elif t and t[0] == "HEIGHT":
            h = int(t[1])
    px = {}
    for i in range(w * h):
        r, g, b, a = body[4 * i:4 * i + 4]
        if a:
            px[(i % w - hot[0], i // w - hot[1])] = (r, g, b, a)
    return px


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    if sys.argv[1] == "compare":
        a, b = read_pam(sys.argv[2]), read_pam(sys.argv[3])
        diff = 0
        for k in set(a) | set(b):
            pa, pb = a.get(k, (0, 0, 0, 0)), b.get(k, (0, 0, 0, 0))
            diff = max(diff, max(abs(u - v) for u, v in zip(pa, pb)))
        print("CURSOR_COMPARE visible_a=%d visible_b=%d max_channel_diff=%d" % (len(a), len(b), diff))
        return 0
    x, f, d = libs()
    if sys.argv[1] == "hide":
        root = x.XDefaultRootWindow(d)
        f.XFixesHideCursor(d, root)
        x.XSync(d, 0)
        print("CURSOR_HIDDEN for %s s" % sys.argv[2], flush=True)
        time.sleep(float(sys.argv[2]))
        f.XFixesShowCursor(d, root)
        x.XSync(d, 0)
        print("CURSOR_SHOWN", flush=True)
        x.XCloseDisplay(d)
        return 0
    if sys.argv[1] == "pointer":
        wins = [ctypes.c_ulong(), ctypes.c_ulong()]
        ints = [ctypes.c_int() for _ in range(4)]
        mask = ctypes.c_uint()
        x.XQueryPointer(d, x.XDefaultRootWindow(d), *[ctypes.byref(v) for v in wins + ints],
                        ctypes.byref(mask))
        print("POINTER %d %d" % (ints[0].value, ints[1].value))
        x.XCloseDisplay(d)
        return 0
    if sys.argv[1] == "image":
        img = f.XFixesGetCursorImage(d)
        if not img:
            print("CURSOR_ERROR XFixesGetCursorImage returned nothing")
            return 3
        c = img.contents
        w, h, hot = c.width, c.height, (c.xhot, c.yhot)
        px = [c.pixels[i] & 0xFFFFFFFF for i in range(w * h)]
        name = c.name.decode(errors="replace") if c.name else ""
        print("CURSOR w=%d h=%d hot=%d,%d serial=%d name=[%s] %s" % (
            w, h, hot[0], hot[1], c.cursor_serial, name, summarize(w, h, hot, px)))
        if len(sys.argv) > 2:
            write_pam(sys.argv[2], w, h, hot, px)
        x.XFree(img)
        x.XCloseDisplay(d)
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
