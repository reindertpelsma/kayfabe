#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""The XOR cursor end to end, no display and no GPU: the real socket, the real
ATTACH validator and cursor path, and the --backend test display, which runs the
SAME driver the Wayland backend runs (nb_xor_eval) and logs what a real backend
would put up:

    TEST cursor: xor patch WxH at X,Y n=N hash 0xH (hover|grab) host pointer hidden
    TEST cursor: XOR composed by the broker (...)            one per change of choice
    TEST cursor: XOR approximated, ... (frame not readable: WHY)
    TEST cursor: shown ... / none (...)                      the ordinary lines

The expected patch is recomputed HERE from the frame bytes this script wrote and
the mask it sent, with the retired kernel's formula (below ^ (mask & 0xffffff)),
so a pass means the broker read the right pixels from the right place and
composed them right -- not merely that something was logged.
"""

import os
import struct
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_cursor as tc  # noqa: E402

check = tc.check
W, H = 160, 100
STRIDE = W * 4 + 32
OFFSET = 64


def px(x, y, gen=0):
    return 0x20000000 | (((x * 5 + y * 3 + gen * 41) & 0xff) << 16) | \
        (((x * 11 + gen) & 0xff) << 8) | ((y * 7 + 3 * gen + 1) & 0xff)


def frame_bytes(w, h, stride, offset, gen=0, total=None):
    buf = bytearray(b"\xaa" * (total if total is not None else offset + stride * h))
    for y in range(h):
        row = struct.pack("<%dI" % w, *[px(x, y, gen) for x in range(w)])
        buf[offset + y * stride: offset + y * stride + len(row)] = row
    return bytes(buf)


def sealed_frame(w=W, h=H, stride=STRIDE, offset=OFFSET, gen=0):
    fd = os.memfd_create("xor-frame", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    os.write(fd, frame_bytes(w, h, stride, offset, gen))
    import fcntl
    fcntl.fcntl(fd, tc.F_ADD_SEALS, tc.F_SEAL_SHRINK)
    return fd


def attach_commit(s, fd, w=W, h=H, stride=STRIDE, offset=OFFSET, shm=True,
                  modifier=0, seq=1):
    tc.send(s, tc.attach_cmd(w, h, stride, tc.XR24, modifier,
                             flags=tc.CMD_F_SHM if shm else 0, offset=offset,
                             seq=seq), [fd])
    tc.send(s, tc.COMMIT)


def ibeam(w=16, h=16):
    on = [(x, 3) for x in (4, 5, 6, 8, 9, 10)] + [(7, y) for y in range(4, 13)] + \
        [(x, 13) for x in (4, 5, 6, 8, 9, 10)]
    return on


def mask_words(w, h, on, colour=0x00ffffff):
    m = [0] * (w * h)
    for x, y in on:
        m[y * w + x] = 0xff000000 | colour
    return m


def expected_patch(mask, w, h, hot, ptr, frame_gen, fw=W, fh=H, tl=None,
                   fpx=px):
    """(pw, ph, tlx, tly, n, hash) of the bbox patch, by the kernel's formula."""
    pts = [(i % w, i // w) for i, v in enumerate(mask) if v >> 24]
    bx, by = min(p[0] for p in pts), min(p[1] for p in pts)
    bw, bh = max(p[0] for p in pts) - bx + 1, max(p[1] for p in pts) - by + 1
    if tl is None:
        tl = (ptr[0] - hot[0], ptr[1] - hot[1])
    tlx, tly = tl[0] + bx, tl[1] + by
    out, n = [], 0
    for iy in range(bh):
        for ix in range(bw):
            m = mask[(by + iy) * w + bx + ix]
            fx, fy = tlx + ix, tly + iy
            v = 0
            if (m >> 24) and 0 <= fx < fw and 0 <= fy < fh:
                v = 0xff000000 | ((fpx(fx, fy, frame_gen) ^ (m & 0xffffff)) & 0xffffff)
                n += 1
            out.append(v)
    return (bw, bh, tlx, tly, n, tc.fnv(out))


def patches(lines):
    import re
    out = []
    for line in lines:
        m = re.search(r"TEST cursor: xor patch (\d+)x(\d+) at (-?\d+),(-?\d+) n=(\d+) "
                      r"hash 0x([0-9a-f]+) \((hover|grab)\) host pointer hidden", line)
        if m:
            w, h, x, y, n, hs, mode = m.groups()
            out.append((int(w), int(h), int(x), int(y), int(n), int(hs, 16), mode))
    return out


def count(lines, text):
    return sum(1 for l in lines if text in l)


def run(b):
    print("-- XOR cursor: composed by the broker from the frame it presents")
    s = b.connect()
    hs = [tc.PKT.unpack(tc.recv_exact(s, tc.PKT.size)) for _ in range(6)]
    check(bool(hs[0][6] & tc.CAP_CURSOR_STREAM), "HELLO advertises CAP_CURSOR_STREAM")
    cw, ch, hot = 16, 16, (4, 8)
    on = ibeam()
    mask = mask_words(cw, ch, on)
    stride_c = cw * 4

    # the frame (a sealed memfd, the wl_shm tier) and the cursor
    fd = sealed_frame(gen=0)
    m = b.mark()
    attach_commit(s, fd, seq=1)
    os.close(fd)
    b.since(m, 0.3)
    cfd = tc.memfd_with(cw, ch, stride_c, 0, mask)
    m = b.mark()
    tc.send(s, tc.cursor_cmd(tc.SET, cw, ch, stride_c, 0, tc.AR24, hot[0], hot[1],
                             flags=tc.F_XOR), [cfd])
    os.close(cfd)
    lines = b.since(m)
    check(count(lines, "XOR composed by the broker") == 1,
          "ONE log line announces the choice (composed) as soon as the cursor and a readable frame meet")
    check(any("sealed shared memory" in l for l in lines),
          "... and names the carrier (sealed shared memory)")
    check(count(lines, "XOR approximated") == 0, "... and says nothing about an approximation")
    check(not patches(lines), "no patch before the pointer has a position")

    # hover: the pointer enters and moves -> the true inversion, host pointer hidden
    m = b.mark()
    b.stdin("p 1", "a 70 40")
    lines = b.since(m)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, (70, 40), 0)
    check(count(lines, "XOR composed") == 0, "the choice is not announced again when the pointer arrives")
    check(len(pt) == 1 and pt[0][:6] == want and pt[0][6] == "hover",
          "hover: the patch is below^colour of the frame's own pixels (want %r got %r)" % (want, pt))

    # moving: a new patch at the new place, no new choice line
    m = b.mark()
    b.stdin("a 71 40")
    lines = b.since(m)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, (71, 40), 0)
    check(len(pt) == 1 and pt[0][:6] == want, "a pointer step recomputes the patch there")
    check(count(lines, "XOR composed") == 0, "... without announcing the choice again")
    m = b.mark()
    b.stdin("a 71 40")
    check(not patches(b.since(m)), "the same position again: nothing new")

    # a pointer faster than the frames: many positions, the last one wins
    m = b.mark()
    path = [(20 + 2 * i, 10 + i // 2) for i in range(60)]
    b.stdin(*["a %d %d" % p for p in path])
    lines = b.since(m, 0.5)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, path[-1], 0)
    check(bool(pt) and pt[-1][:6] == want,
          "60 pointer moves between frames: the patch ends exactly at the last (frame 0) position")
    check(len(pt) <= len(path), "... at most one patch line per position (%d)" % len(pt))

    # a new FRAME (different pixels), the pointer still: the patch follows the frame
    fd = sealed_frame(gen=3)
    m = b.mark()
    attach_commit(s, fd, seq=2)
    os.close(fd)
    lines = b.since(m)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, path[-1], 3)
    check(len(pt) == 1 and pt[0][:6] == want,
          "a new frame recomputes the patch from THAT frame (hash 0x%08x)" % want[5])
    check(count(lines, "XOR composed") == 0, "... the choice is not announced per frame")
    # the same frame again (static desktop): nothing at all
    fd = sealed_frame(gen=3)
    m = b.mark()
    attach_commit(s, fd, seq=3)
    os.close(fd)
    check(not patches(b.since(m)), "an identical next frame changes nothing under the cursor: no patch line")

    # edges: partly off the frame, and the far corner, clamped
    for ptr in ((0, 0), (W - 1, H - 1), (W - 1, 0), (0, H - 1), (3, 60)):
        m = b.mark()
        b.stdin("a %d %d" % ptr)
        pt = patches(b.since(m))
        want = expected_patch(mask, cw, ch, hot, ptr, 3)
        check(len(pt) == 1 and pt[0][:6] == want,
              "pointer at %r: the patch is clipped to the frame (n=%d)" % (ptr, want[4]))
    m = b.mark()
    b.stdin("a 5000 5000")
    pt = patches(b.since(m))
    want = expected_patch(mask, cw, ch, hot, (W - 1, H - 1), 3)
    check(len(pt) == 1 and pt[0][:6] == want, "a pointer beyond the frame is clamped to its last pixel")

    # a RESIZED frame: other geometry, the patch is read against the new one
    w2, h2 = 120, 80
    s2 = w2 * 4
    fd = sealed_frame(w2, h2, s2, 0, gen=5)
    m = b.mark()
    attach_commit(s, fd, w2, h2, s2, 0, seq=4)
    os.close(fd)
    b.stdin("a 50 30")
    lines = b.since(m)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, (50, 30), 5, fw=w2, fh=h2)
    check(bool(pt) and pt[-1][:6] == want, "a resized frame: the patch is composed against the new frame")
    m = b.mark()
    b.stdin("a 119 79")
    pt = patches(b.since(m))
    want = expected_patch(mask, cw, ch, hot, (119, 79), 5, fw=w2, fh=h2)
    check(len(pt) == 1 and pt[0][:6] == want, "... and clipped to the NEW size")

    # the frame stops being readable (a plain memfd, not F_SHM, not a dma-buf)
    fd = tc.plain_memfd(W * H * 4 + 4096, seal=True)
    m = b.mark()
    attach_commit(s, fd, W, H, W * 4, 0, shm=False, seq=5)
    os.close(fd)
    lines = b.since(m)
    check(count(lines, "XOR approximated") == 1, "an unreadable frame: ONE 'approximated' line")
    check(any("frame not readable" in l for l in lines), "... with the reason")
    st = tc.shown(lines)
    want_apx = tc.fnv(tc.expected_substitute(cw, ch, on))
    check(st is not None and st[0] == "shown" and st[6] == want_apx,
          "... and the host pointer shows the approximation again (black fill, white outline)")
    m = b.mark()
    for i in range(5):
        fd = tc.plain_memfd(W * H * 4 + 4096, seal=True)
        attach_commit(s, fd, W, H, W * 4, 0, shm=False, seq=6 + i)
        os.close(fd)
    lines = b.since(m)
    check(count(lines, "XOR approximated") == 0 and count(lines, "XOR composed") == 0,
          "five more unreadable frames: no further choice lines")

    # readable again: composed, announced once
    fd = sealed_frame(gen=7)
    m = b.mark()
    attach_commit(s, fd, seq=20)
    os.close(fd)
    b.stdin("a 70 40")
    lines = b.since(m)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, (70, 40), 7)
    check(count(lines, "XOR composed by the broker") == 1, "readable again: composed, said once")
    check(bool(pt) and pt[-1][:6] == want, "... and the patch is the new frame's")

    # restore: a plain cursor -> the host pointer shows it, no patch
    arrow = tc.image(8, 8, 3)
    m = b.mark()
    cfd = tc.memfd_with(8, 8, 32, 0, arrow)
    tc.send(s, tc.cursor_cmd(tc.SET, 8, 8, 32, 0, tc.AR24, 1, 1), [cfd])
    os.close(cfd)
    lines = b.since(m)
    st = tc.shown(lines)
    check(st is not None and st[0] == "shown" and st[6] == tc.fnv([tc.premultiplied(v) for v in arrow]),
          "I-beam -> arrow: the host pointer is the arrow again")
    check(not patches(lines), "... and the patch is gone")
    # and back: the I-beam composes again
    m = b.mark()
    cfd = tc.memfd_with(cw, ch, stride_c, 0, mask)
    tc.send(s, tc.cursor_cmd(tc.SET, cw, ch, stride_c, 0, tc.AR24, hot[0], hot[1],
                             flags=tc.F_XOR), [cfd])
    os.close(cfd)
    lines = b.since(m)
    pt = patches(lines)
    check(len(pt) == 1 and pt[0][:6] == expected_patch(mask, cw, ch, hot, (70, 40), 7),
          "arrow -> I-beam: the patch is back")
    # HIDE / SHOW
    m = b.mark()
    tc.send(s, tc.cursor_cmd(tc.HIDE))
    check(tc.shown(b.since(m)) == ("none", "hidden"), "HIDE: nothing at all")
    m = b.mark()
    tc.send(s, tc.cursor_cmd(tc.SHOW))
    check(len(patches(b.since(m))) == 1, "SHOW: the patch is back")
    # the pointer leaves the window: the software cursor goes with it
    m = b.mark()
    b.stdin("p 0")
    st = tc.shown(b.since(m))
    check(st == ("none", "outside"), "pointer outside the window: no patch, host pointer not ours (got %r)" % (st,))
    m = b.mark()
    b.stdin("p 1", "a 70 40")
    check(len(patches(b.since(m))) == 1, "the pointer returns: the patch with it")

    # grab: the patch at the MOVE position (the guest's own)
    tc.send(s, tc.move_cmd(30, 20))
    b.stdin("f 1")
    m = b.mark()
    b.stdin(*tc.GRAB)
    lines = b.since(m)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, None, 7, tl=(30, 20))
    check(bool(pt) and pt[-1][:6] == want and pt[-1][6] == "grab",
          "grab: the patch sits at the last MOVE (want %r got %r)" % (want, pt[-1:] if pt else pt))
    m = b.mark()
    tc.send(s, tc.move_cmd(-3, 95))
    pt = patches(b.since(m))
    want = expected_patch(mask, cw, ch, hot, None, 7, tl=(-3, 95))
    check(len(pt) == 1 and pt[0][:6] == want, "grab: a MOVE off the left/bottom edge is clipped")
    m = b.mark()
    tc.send(s, tc.move_cmd(2000, 2000))
    lines = b.since(m)
    check(any("nothing of the cursor is on the frame" in l for l in lines),
          "grab: wholly off the frame composes to nothing")
    m = b.mark()
    b.stdin(*tc.GRAB)
    lines = b.since(m)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, (70, 40), 7)
    check(bool(pt) and pt[-1][:6] == want and pt[-1][6] == "hover",
          "ending the grab: the patch is back at the host pointer (hover), not at the MOVE")
    s.close()
    time.sleep(0.3)


def run_dmabuf(b):
    print("-- XOR cursor on a LINEAR dma-buf (udmabuf): only the cursor's rows are read")
    # a tightly packed frame, page-multiple, in a udmabuf
    import fcntl
    dev = None
    try:
        dev = os.open("/dev/udmabuf", os.O_RDWR | os.O_CLOEXEC)
    except OSError:
        print("  SKIP no /dev/udmabuf")
        return
    mfd = os.memfd_create("udma-frame", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
    stride = W * 4
    total = 4096 * ((stride * H + 4095) // 4096)
    os.write(mfd, frame_bytes(W, H, stride, 0, gen=2, total=total))
    fcntl.fcntl(mfd, tc.F_ADD_SEALS, tc.F_SEAL_SHRINK)
    arg = bytearray(struct.pack("<IIQQ", mfd, 1, 0, total))
    try:
        dfd = fcntl.ioctl(dev, tc.UDMABUF_CREATE, arg, True)
    except OSError as e:
        print("  SKIP UDMABUF_CREATE: %s" % e)
        return
    finally:
        os.close(mfd)
        os.close(dev)
    s = b.connect()
    for _ in range(6):
        tc.recv_exact(s, tc.PKT.size)
    cw, ch, hot = 16, 16, (4, 8)
    mask = mask_words(cw, ch, ibeam(), colour=0x00123456)
    cfd = tc.memfd_with(cw, ch, cw * 4, 0, mask)
    tc.send(s, tc.cursor_cmd(tc.SET, cw, ch, cw * 4, 0, tc.AR24, hot[0], hot[1],
                             flags=tc.F_XOR), [cfd])
    os.close(cfd)
    m = b.mark()
    attach_commit(s, dfd, W, H, stride, 0, shm=False, modifier=0, seq=1)
    b.stdin("p 1", "a 40 30")
    lines = b.since(m, 0.4)
    pt = patches(lines)
    want = expected_patch(mask, cw, ch, hot, (40, 30), 2)
    check(any("LINEAR dma-buf, exporter udmabuf" in l for l in lines),
          "the choice line names the carrier: LINEAR dma-buf, exporter udmabuf")
    check(bool(pt) and pt[-1][:6] == want,
          "dma-buf tier: the patch is below^colour of the frame read through the mapping")
    m = b.mark()
    b.stdin("a 41 31")
    pt = patches(b.since(m))
    want = expected_patch(mask, cw, ch, hot, (41, 31), 2)
    check(len(pt) == 1 and pt[0][:6] == want, "dma-buf tier: follows the pointer")
    os.close(dfd)
    s.close()
    time.sleep(0.3)


def run_tier(b):
    print("-- the carrier of the frames is said once per change, truthfully")
    s = b.connect()
    for _ in range(6):
        tc.recv_exact(s, tc.PKT.size)
    m = b.mark()
    fd = sealed_frame()
    attach_commit(s, fd, seq=1)
    os.close(fd)
    lines = b.since(m)
    check(count(lines, "frames now arrive as plain shared memory (F_SHM)") == 1,
          "the first F_SHM frame: one line saying the VMM chose shared memory")
    check(not any("refused every dma-buf layout" in l for l in lines),
          "... and nothing claims the display refused a layout")
    check(any("this display has refused 0 dma-buf layout(s)" in l for l in lines),
          "... the number of refusals the broker actually recorded is stated (0)")
    m = b.mark()
    for i in range(3):
        fd = sealed_frame(gen=i)
        attach_commit(s, fd, seq=2 + i)
        os.close(fd)
    check(count(b.since(m), "frames now arrive") == 0, "more shared-memory frames: no further line")
    size = 4096 * ((W * 4 * H + 4095) // 4096)
    dfd = tc.udmabuf(size)
    if dfd is None:
        print("  SKIP no udmabuf for the dma-buf half")
    else:
        m = b.mark()
        attach_commit(s, dfd, W, H, W * 4, 0, shm=False, modifier=0, seq=9)
        os.close(dfd)
        lines = b.since(m)
        check(count(lines, "frames arrive as dma-buf (zero-copy) again, after shared memory") == 1,
              "back to a dma-buf: one line, saying it is again")
    s.close()
    time.sleep(0.3)
    s = b.connect()
    for _ in range(6):
        tc.recv_exact(s, tc.PKT.size)
    m = b.mark()
    fd = sealed_frame()
    attach_commit(s, fd, seq=1)
    os.close(fd)
    check(count(b.since(m), "frames now arrive as plain shared memory (F_SHM)") == 1,
          "a new connection says it afresh")
    s.close()
    time.sleep(0.3)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: test_xor.py /path/to/broker")
    tc.BROKER = os.path.abspath(sys.argv[1])
    with tempfile.TemporaryDirectory() as tmp:
        b = tc.Broker(os.path.join(tmp, "broker.sock"))
        try:
            run(b)
        finally:
            b.stop()
        b = tc.Broker(os.path.join(tmp, "broker2.sock"))
        try:
            run_dmabuf(b)
            run_tier(b)
        finally:
            b.stop()
    if tc.FAILS:
        print("%d XOR check(s) FAILED" % len(tc.FAILS))
        sys.exit(1)
    print("xor tests passed")


if __name__ == "__main__":
    main()
