#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""EV_TICK, the lock-step frame requests and CMD_NO_CHANGE over the real
socket, no display required.

Drives the broker's --backend test, whose clock is VIRTUAL: it moves only when
a stdin line says so (`t <present_ns> <refresh_ns>` is one display feedback,
`F <n> <refresh_ns> <step_ns>` is n of them step_ns apart -- a flood when the
step is a fraction of the refresh --, `P` is "the last frame was presented",
`S` logs the tick counters), so every number below is exact, not "about".

Asserted: the handshake an old VMM sees is byte-for-byte today's; the opt-in;
the tick sequence and the WANT_FRAME rules end to end; the reply rules (COMMIT
.seq, NO_CHANGE) and every violation row of the design table (the broker must
hang up on the offender); the decimation and the mode rate the guest is told;
and the FLOODS: feedbacks at 10x, 100x and 1000x the refresh rate, steady and
in bursts, with a peer that drains and with one that never reads (the broker
keeps running, its queue stays at two ticks, the rest are counted), a hostile
VMM flooding replies, and every other broker->VMM message kind interleaved so
that a tick flood cannot starve them and they cannot starve ticks.
"""

import os
import re
import socket
import struct
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_cursor as tc   # noqa: E402
import test_xor as tx      # noqa: E402

check = tc.check
PKT, CMD = tc.PKT, tc.CMD
TICK = struct.Struct("<HHIIIQ")
assert TICK.size == 24

EV_HELLO, EV_SURFACE, EV_FRAME, EV_RELEASE, EV_FOCUS, EV_GRAB = 1, 2, 3, 4, 11, 10
EV_KEY, EV_BYE, EV_TICK = 5, 13, 18
CMD_COMMIT, CMD_CAPS, CMD_NO_CHANGE = 2, 5, 8
CAP_TICK = 1 << 13
CLIENT_TICK = 1 << 1
WANT, CLOCK_ONLY, MASK = 1 << 28, 1 << 29, 0x0FFFFFFF
RESERVED = 0xC0000000
P60 = 16666667
MS = 1000000


_VT = [10**9]


def vt(b, span=0):
    """A fresh, strictly later point on the broker's virtual clock (and the
    clock moved there): every test connects AFTER it, so time never runs back."""
    base = _VT[0]
    _VT[0] += span + 20 * 10**9
    b.feed("N %d" % base)
    time.sleep(0.05)
    return base


def paced(b, lines, gap=0.004):
    """Feed lines one at a time, giving the broker's loop time to flush between
    them: a tick queued behind another that was not flushed yet is REPLACED (the
    queue rule), so a test that wants to see every tick on the wire must let
    each one leave first."""
    for line in lines:
        b.feed(line)
        time.sleep(gap)


def paced_roundtrip(b, sock, lines, div):
    """Synchronize on each accepted EV_TICK, so queue coalescing cannot race this fixture."""
    packets = []
    sock.settimeout(0.6)  # the existing decimation read bound, not a pacing delay
    for index, line in enumerate(lines):
        b.feed(line)
        if index % div == 0:
            while True:
                packet = tc.recv_exact(sock, PKT.size)
                packets.append(packet)
                if struct.unpack_from("<H", packet)[0] == EV_TICK:
                    break
    return packets


def caps(bits, height=0, **kw):
    f = dict(stride=0, offset=0, fourcc=0, modifier=0, seq=0, reserved=0)
    f.update(kw)
    return CMD.pack(CMD_CAPS, 0, bits, height, f["stride"], f["offset"],
                    f["fourcc"], f["modifier"], f["seq"], f["reserved"])


def commit(seq=0, **kw):
    f = dict(width=0, height=0, stride=0, offset=0, fourcc=0, modifier=0,
             flags=0, reserved=0)
    f.update(kw)
    return CMD.pack(CMD_COMMIT, f["flags"], f["width"], f["height"], f["stride"],
                    f["offset"], f["fourcc"], f["modifier"], seq, f["reserved"])


def nochange(req, **kw):
    f = dict(height=0, stride=0, offset=0, fourcc=0, modifier=0, seq=0,
             flags=0, reserved=0)
    f.update(kw)
    return CMD.pack(CMD_NO_CHANGE, f["flags"], req, f["height"], f["stride"],
                    f["offset"], f["fourcc"], f["modifier"], f["seq"],
                    f["reserved"])


def attach(s, seq=1):
    fd = tx.sealed_frame()
    tc.send(s, tc.attach_cmd(tx.W, tx.H, tx.STRIDE, tc.XR24, 0,
                             flags=tc.CMD_F_SHM, offset=tx.OFFSET, seq=seq), [fd])
    os.close(fd)


def frame(s, req):
    """A frame answering request `req`: ATTACH (advisory seq) + COMMIT(seq=req)."""
    attach(s)
    tc.send(s, commit(req))


class Tick:
    """A decoded EV_TICK."""

    def __init__(self, raw):
        (self.type, self.flags, self.seq, self.tick_seq, self.word,
         self.present) = TICK.unpack(raw)
        self.period = self.word & MASK
        self.want = bool(self.word & WANT)
        self.clock_only = bool(self.word & CLOCK_ONLY)
        self.reserved = self.word & RESERVED


def read_raw(sock, timeout=0.3):
    """Every complete 24-byte packet available within `timeout`."""
    out = []
    sock.settimeout(timeout)
    buf = b""
    try:
        while True:
            part = sock.recv(65536)
            if not part:
                break
            buf += part
            sock.settimeout(0.05)
    except (socket.timeout, OSError):
        pass
    for i in range(0, len(buf) - len(buf) % 24, 24):
        out.append(buf[i:i + 24])
    return out


def ticks_of(raws):
    return [Tick(r) for r in raws if struct.unpack_from("<H", r)[0] == EV_TICK]


def types_of(raws):
    return [struct.unpack_from("<H", r)[0] for r in raws]


class Drainer:
    """Reads the socket on a thread, so the broker never waits for the peer."""

    def __init__(self, sock):
        self.sock = sock
        self.raws = []
        self.buf = b""
        self.stop = False
        self.lock = threading.Lock()
        self.t = threading.Thread(target=self.run, daemon=True)
        self.t.start()

    def run(self):
        self.sock.settimeout(0.05)
        while not self.stop:
            try:
                part = self.sock.recv(1 << 20)
            except socket.timeout:
                continue
            except OSError:
                return
            if not part:
                return
            with self.lock:
                self.buf += part
                n = len(self.buf) - len(self.buf) % 24
                for i in range(0, n, 24):
                    self.raws.append(self.buf[i:i + 24])
                self.buf = self.buf[n:]

    def take(self):
        with self.lock:
            r, self.raws = self.raws, []
        return r

    def finish(self):
        time.sleep(0.3)
        self.stop = True
        self.t.join(2)
        return self.take()


class B(tc.Broker):
    """The test broker, with a counter barrier: `S` is logged after everything
    written before it was processed."""

    def feed(self, *lines):
        self.proc.stdin.write("".join(l + "\n" for l in lines))
        self.proc.stdin.flush()

    def stats(self, timeout=20.0):
        mark = self.mark()
        self.feed("S")
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            with self.lock:
                for line in self.lines[mark:]:
                    if "TEST tick stats:" in line:
                        return {k: int(v) for k, v in
                                re.findall(r"(\w+)=(\d+)", line)}
            time.sleep(0.01)
        raise RuntimeError("no tick stats line")


def connect(b, opt_in=True, height=0):
    s = b.connect()
    hs = [PKT.unpack(tc.recv_exact(s, PKT.size)) for _ in range(6)]
    if opt_in:
        s.sendall(caps(CLIENT_TICK, height))
    return s, hs


def violated(s):
    return tc.closed(s, 1.5)


def alive(b):
    """Still serving: a fresh connection gets its handshake."""
    s = b.connect()
    try:
        return len([PKT.unpack(tc.recv_exact(s, PKT.size)) for _ in range(6)]) == 6
    except Exception:
        return False
    finally:
        s.close()


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: test_tick.py /path/to/broker")
    tc.BROKER = os.path.abspath(sys.argv[1])
    tx.tc.BROKER = tc.BROKER
    with tempfile.TemporaryDirectory() as tmp:
        b = B(os.path.join(tmp, "broker.sock"))
        try:
            old_vmm(b)
            opt_in(b)
            lock_step(b)
            replies(b)
            failed_commit(b)
            decimation(b)
            violations(b)
            floods(b)
            interleaved(b)
            never_reads(b)
            hostile_replies(b)
            reconnect(b)
        finally:
            b.stop()
        old_broker(tmp)
    if tc.FAILS:
        print("%d tick check(s) FAILED" % len(tc.FAILS))
        sys.exit(1)
    print("tick tests passed")


def old_vmm(b):
    print("-- an old VMM (no CAPS) sees today's exchange and no EV_TICK")
    t0 = vt(b)
    s, hs = connect(b, opt_in=False)
    check([p[0] for p in hs] == [EV_HELLO, EV_SURFACE, EV_FOCUS, EV_GRAB, EV_FRAME, 17],
          "the six handshake packets are unchanged")
    check(bool(hs[0][6] & CAP_TICK), "HELLO advertises CAP_TICK (bit 13)")
    check(hs[0][6] & ~(CAP_TICK | 0x1FFF) == 0, "and no other new bit")
    b.feed("t %d %d" % (t0 + P60, P60), "t %d %d" % (t0 + 2 * P60, P60))
    frame(s, 77)                      # COMMIT.seq is advisory for an old VMM
    raws = read_raw(s, 0.5)
    check(ticks_of(raws) == [], "no EV_TICK to a VMM that did not opt in")
    check(EV_RELEASE in types_of(raws) and EV_FRAME in types_of(raws),
          "the legacy RELEASE + FRAME exchange is intact")
    st = b.stats()
    check(st["on"] == 0 and st["ticks"] == 0 and st["feedback"] == 0,
          "tick counters stay at zero")
    s.close()


def opt_in(b):
    print("-- opt-in: CAPS(CLIENT_TICK) -> the priming tick")
    vt(b)
    s, _ = connect(b, height=60)
    raws = read_raw(s, 0.4)
    t = ticks_of(raws)
    check(len(t) == 1, "exactly one tick follows the opt-in (the priming tick)")
    if t:
        p = t[0]
        check(p.tick_seq == 1, "the priming tick is number 1")
        check(p.want, "the priming tick carries WANT_FRAME")
        check(p.clock_only, "and CLOCK_ONLY (it is not a display timestamp)")
        check(p.reserved == 0, "the reserved bits are zero")
        check(p.flags & 0xFFF8 == 0, "the flags are only the mirrored state bits")
        check(p.period == 0, "the priming period is unknown (0)")
    s.close()


def lock_step(b):
    print("-- the tick sequence and WANT_FRAME, end to end")
    t0 = vt(b, 2 * 10**9)
    s, _ = connect(b, height=75)
    prime = ticks_of(read_raw(s, 0.3))
    check(len(prime) == 1 and prime[0].want, "priming tick opens request 1")
    paced(b, ["t %d %d" % (t0 + i * P60, P60) for i in range(6)])
    t = ticks_of(read_raw(s, 0.5))
    check([x.tick_seq for x in t] == [2, 3, 4, 5, 6, 7], "tick_seq counts 2..7")
    check(not any(x.want for x in t), "no second request while request 1 is open")
    check(all(x.period == P60 for x in t), "the period is the host's (60 Hz, undivided)")
    check([x.present for x in t] == [t0 + i * P60 for i in range(6)],
          "present_ns is the feedback's, exact")
    check(all(not x.clock_only for x in t), "a compositor timestamp is not CLOCK_ONLY")
    check(all(a.seq < b_.seq for a, b_ in zip(t, t[1:])), "the packet counter has no reuse")
    # answer request 1 with a frame: no new request until it is presented
    frame(s, 1)
    time.sleep(0.2)
    read_raw(s, 0.2)
    paced(b, ["t %d %d" % (t0 + (6 + i) * P60, P60) for i in range(3)])
    t = ticks_of(read_raw(s, 0.4))
    check(len(t) == 3 and not any(x.want for x in t),
          "an answered, un-presented frame blocks the next request")
    b.feed("P", "t %d %d" % (t0 + 9 * P60, P60))
    t = ticks_of(read_raw(s, 0.4))
    check(len(t) == 1 and t[0].want and t[0].tick_seq == 11,
          "after the presentation the next tick wants (request 11)")
    # NO_CHANGE closes it at once
    s.sendall(nochange(11))
    time.sleep(0.2)
    b.feed("t %d %d" % (t0 + 10 * P60, P60))
    t = ticks_of(read_raw(s, 0.4))
    check(len(t) == 1 and t[0].want and t[0].tick_seq == 12,
          "NO_CHANGE closes the request: the next tick wants (request 12)")
    # the 100 ms timeout: exactly at the edge
    b.feed("N %d" % (t0 + 10 * P60 + 100 * MS - 1),
           "t %d %d" % (t0 + 10 * P60 + 100 * MS - 1, P60))
    t = ticks_of(read_raw(s, 0.3))
    check(len(t) == 1 and not t[0].want, "1 ns before the request timeout: still open")
    b.feed("t %d %d" % (t0 + 10 * P60 + 100 * MS + P60, P60))
    t = ticks_of(read_raw(s, 0.3))
    check(len(t) == 1 and t[0].want, "past the timeout a new request opens")
    st = b.stats()
    check(st["req_timeouts"] == 1, "one request timeout counted (%d)" % st["req_timeouts"])
    s.close()


def replies(b):
    print("-- late, unsolicited and duplicate replies")
    t0 = vt(b)
    s, _ = connect(b, height=75)
    read_raw(s, 0.3)
    b.feed(*["t %d %d" % (t0 + i * P60, P60) for i in range(3)])
    read_raw(s, 0.3)
    before = b.stats()
    frame(s, 0)                       # unsolicited: shown, counted
    s.sendall(nochange(2))            # issued, never requested: late
    s.sendall(nochange(2))
    time.sleep(0.3)
    after = b.stats()
    check(after["unsolicited"] == before["unsolicited"] + 1, "COMMIT seq 0 is counted unsolicited")
    check(after["late"] == before["late"] + 2, "issued but not open: late, counted")
    check(after["open"] == before["open"] and after["open"] == 1,
          "none of them closed the open request")
    frame(s, 1)                       # the open one
    time.sleep(0.2)
    after2 = b.stats()
    check(after2["open"] == 0 and after2["unpresented"] == 1, "the open request is answered")
    frame(s, 1)                       # a duplicate answer: late, shown
    time.sleep(0.2)
    check(b.stats()["late"] == after["late"] + 1, "a duplicate answer is late")
    check(b.stats()["on"] == 1, "the broker serves on")
    s.close()


def failed_commit(b):
    print("-- a COMMIT the display could not show answers the request and blocks nothing")
    t0 = vt(b)
    s, _ = connect(b, height=75)
    read_raw(s, 0.2)                  # the priming tick opened request 1
    s.sendall(commit(1))              # no ATTACH before it: the backend answers -ENOENT
    time.sleep(0.2)
    b.feed("t %d %d" % (t0 + P60, P60))
    t = ticks_of(read_raw(s, 0.3))
    check(len(t) == 1 and t[0].want,
          "the very next tick wants a frame again (nothing was shown, nothing to wait for)")
    st = b.stats()
    check(st["unpresented"] == 0, "and no frame is waiting for a presentation")
    s.close()


def decimation(b):
    print("-- decimation, and the mode rate the guest is told")
    for host, cap, div, mode in ((144, 75, 2, 72000), (240, 75, 4, 60000),
                                 (144, 60, 3, 48000), (165, 75, 3, 55000),
                                 (60, 75, 1, 60000)):
        t0 = vt(b)
        s, hs = connect(b, height=cap)
        read_raw(s, 0.2)
        period = int(1e9 / host)
        raws = paced_roundtrip(b, s, ["t %d %d" % (t0 + i * period, period) for i in range(host)], div)
        raws += read_raw(s, 0.6)
        t = ticks_of(raws)
        check(len(t) == host // div + (1 if host % div else 0),
              "%d Hz cap %d: %d ticks of %d feedbacks (div %d), got %d"
              % (host, cap, host // div, host, div, len(t)))
        check(all(x.period == period * div for x in t),
              "%d Hz cap %d: the period on the wire is host x %d" % (host, cap, div))
        surf = [PKT.unpack(r) for r in raws if struct.unpack_from("<H", r)[0] == EV_SURFACE]
        check(any(p[5] == mode for p in surf),
              "%d Hz cap %d: EV_SURFACE tells the guest %d mHz (got %r)"
              % (host, cap, mode, [p[5] for p in surf]))
        s.close()
        time.sleep(0.1)
    print("-- a host refresh change re-divides and re-advertises")
    t0 = vt(b)
    s, _ = connect(b, height=75)
    read_raw(s, 0.2)
    p144 = int(1e9 / 144)
    paced(b, ["t %d %d" % (t0 + i * p144, p144) for i in range(8)])
    raws = read_raw(s, 0.4)
    check(len(ticks_of(raws)) == 4, "144 Hz: every second feedback")
    t1 = t0 + 8 * p144 + P60
    paced(b, ["t %d %d" % (t1 + i * P60, P60) for i in range(4)])
    raws = read_raw(s, 0.4)
    check(len(ticks_of(raws)) == 4, "then 60 Hz: every feedback")
    surf = [PKT.unpack(r)[5] for r in raws if struct.unpack_from("<H", r)[0] == EV_SURFACE]
    check(60000 in surf, "and the mode follows: 60000 mHz (got %r)" % surf)
    s.close()


def violations(b):
    print("-- every violation row: the offender is disconnected")
    cases = [
        ("NO_CHANGE without CLIENT_TICK", False, lambda s: s.sendall(nochange(1))),
        ("NO_CHANGE request 0", True, lambda s: s.sendall(nochange(0))),
        ("NO_CHANGE for a request never issued", True, lambda s: s.sendall(nochange(99))),
        ("NO_CHANGE with a far-future request", True, lambda s: s.sendall(nochange(0x80000000))),
        ("NO_CHANGE with flags", True, lambda s: s.sendall(nochange(1, flags=1))),
        ("NO_CHANGE with a height", True, lambda s: s.sendall(nochange(1, height=1))),
        ("NO_CHANGE with a stride", True, lambda s: s.sendall(nochange(1, stride=4))),
        ("NO_CHANGE with a seq", True, lambda s: s.sendall(nochange(1, seq=1))),
        ("NO_CHANGE with reserved1", True, lambda s: s.sendall(nochange(1, reserved=1))),
        ("NO_CHANGE with an fd", True, lambda s: tc.send(s, nochange(1), [os.dup(0)])),
        ("COMMIT.seq never issued", True, lambda s: s.sendall(commit(9999))),
        ("CAPS height without CLIENT_TICK", False, lambda s: s.sendall(caps(0, 60))),
        ("CAPS height 23 (below 24)", False, lambda s: s.sendall(caps(CLIENT_TICK, 23))),
        ("CAPS height 76 (above 75)", False, lambda s: s.sendall(caps(CLIENT_TICK, 76))),
        ("CAPS height 100000", False, lambda s: s.sendall(caps(CLIENT_TICK, 100000))),
        ("CAPS with a stride", False, lambda s: s.sendall(caps(CLIENT_TICK, 0, stride=1))),
    ]
    for name, opted, send in cases:
        vt(b)
        s, _ = connect(b, opt_in=opted, height=60)
        time.sleep(0.1)
        read_raw(s, 0.15)
        send(s)
        check(violated(s), "%s: disconnected" % name)
        s.close()
    check(alive(b), "after all of them the broker serves on")
    print("-- and what is NOT a violation")
    t0 = vt(b)
    s, _ = connect(b, height=0)
    for name, msg in (("CAPS height 24", caps(CLIENT_TICK, 24)),
                      ("CAPS height 75", caps(CLIENT_TICK, 75)),
                      ("CAPS height 0 (= 75)", caps(CLIENT_TICK, 0))):
        s.sendall(msg)
        time.sleep(0.1)
        check(not tc.closed(s, 0.2), "%s: accepted" % name)
    s.sendall(caps(0, 0))             # withdrawn
    time.sleep(0.1)
    read_raw(s, 0.2)
    b.feed("t %d %d" % (t0 + P60, P60))
    check(ticks_of(read_raw(s, 0.3)) == [], "a withdrawn CLIENT_TICK stops the ticks")
    s.close()


def floods(b):
    print("-- feedback floods at 10x, 100x and 1000x the refresh, with a draining peer")
    for mult in (10, 100, 1000):
        vt(b)
        s, _ = connect(b, height=75)
        d = Drainer(s)
        time.sleep(0.2)
        d.take()
        step = P60 // mult
        n = 60 * mult                         # one simulated second
        base = b.stats()
        b.feed("F %d %d %d" % (n, P60, step))
        st = b.stats()
        raws = d.finish()
        t = ticks_of(raws)
        accepted = st["ticks"] - base["ticks"]
        fast = st["fast"] - base["fast"]
        check(len(t) <= accepted and len(t) >= 1,
              "%dx: the peer got %d ticks of %d emitted" % (mult, len(t), accepted))
        check(t and t[-1].tick_seq == st["last_seq"],
              "%dx: the newest tick is delivered" % mult)
        check(accepted <= 121 and accepted >= 59,
              "%dx: %d feedbacks -> %d ticks (at most 2 per period)" % (mult, n, accepted))
        check(fast >= n - 121 - 1,
              "%dx: %d of %d feedbacks dropped as too fast (counted)" % (mult, fast, n))
        check(all(a.tick_seq < b_.tick_seq for a, b_ in zip(t, t[1:])),
              "%dx: tick_seq strictly increases on the wire" % mult)
        check(st["feedback"] - base["feedback"] == n, "%dx: every feedback was counted" % mult)
        print("   %dx: %d feedbacks -> %d ticks emitted, %d dropped too-fast, %d coalesced, peer saw %d"
              % (mult, n, accepted, fast, st["coalesced"] - base["coalesced"], len(t)))
        s.close()
        time.sleep(0.1)
    print("-- bursts: 1000 feedbacks at one instant, repeated")
    t0 = vt(b, 3 * 10**9)
    s, _ = connect(b, height=75)
    d = Drainer(s)
    time.sleep(0.2)
    d.take()
    base = b.stats()
    for burst in range(20):
        b.feed("N %d" % (t0 + (burst + 1) * 100 * MS))
        b.feed("F 1000 %d 0" % P60)
    st = b.stats()
    t = ticks_of(d.finish())
    check(st["ticks"] - base["ticks"] == 20,
          "20 bursts of 1000 identical-instant feedbacks tick once each (%d)"
          % (st["ticks"] - base["ticks"]))
    check(st["fast"] - base["fast"] == 19980, "the 19980 duplicates are counted (%d)" % (st["fast"] - base["fast"]))
    s.close()


def interleaved(b):
    print("-- every other message kind interleaved with a tick flood")
    t0 = vt(b, 5 * 10**9)
    s, _ = connect(b, height=75)
    d = Drainer(s)
    time.sleep(0.2)
    d.take()
    base = b.stats()
    nkeys = 200
    step = P60 // 100
    b.feed("f 0")                             # a known starting state
    time.sleep(0.2)
    d.take()
    for i in range(nkeys):
        b.feed("N %d" % (t0 + i * 100 * step))
        b.feed("F 100 %d %d" % (P60, step))
        b.feed("f %d" % ((i + 1) % 2))        # EV_FOCUS: a real change each time
        b.feed("d 226 %d 3" % i)              # EV_DEVICE
        time.sleep(0.003)
    # the VMM talks too: replies and frames, producing RELEASE + FRAME
    for i in range(20):
        frame(s, 0)
    st = b.stats()
    raws = d.finish()
    types = types_of(raws)
    ticks = ticks_of(raws)
    check(types.count(EV_FOCUS) == nkeys,
          "all %d FOCUS events arrived despite the flood (got %d)" % (nkeys, types.count(EV_FOCUS)))
    check(types.count(17) == nkeys, "all %d EV_DEVICE events arrived (got %d)" % (nkeys, types.count(17)))
    check(types.count(EV_RELEASE) == 20 and types.count(EV_FRAME) >= 20,
          "the 20 RELEASE + FRAME answers arrived (got %d/%d)" % (types.count(EV_RELEASE), types.count(EV_FRAME)))
    check(len(ticks) >= nkeys // 4,
          "ticks were not starved by them: %d ticks for %d refresh periods" % (len(ticks), nkeys))
    check(ticks and ticks[-1].tick_seq == st["last_seq"],
          "the NEWEST tick always arrives (last on the wire %d, last emitted %d)"
          % (ticks[-1].tick_seq if ticks else -1, st["last_seq"]))
    check(all(a.tick_seq < b_.tick_seq for a, b_ in zip(ticks, ticks[1:])), "tick_seq increases")
    seqs = [struct.unpack_from("<I", r, 4)[0] for r in raws]
    check(all(a < b_ for a, b_ in zip(seqs, seqs[1:])), "the packet counter increases across all kinds")
    check(st["ticks"] - base["ticks"] >= nkeys, "emitted %d ticks" % (st["ticks"] - base["ticks"]))
    check(len(ticks) + (st["coalesced"] - base["coalesced"]) <= st["ticks"] - base["ticks"] + 1,
          "received + coalesced never exceeds emitted")
    print("   interleaved: %d ticks, %d focus, %d device, %d release, %d frame"
          % (len(ticks), types.count(EV_FOCUS), types.count(17), types.count(EV_RELEASE), types.count(EV_FRAME)))
    s.close()


def never_reads(b):
    print("-- a peer that never reads: the broker's queue stays bounded")
    vt(b, 110 * 10**9)
    s, _ = connect(b, height=75)
    time.sleep(0.2)
    base = b.stats()
    # 1000x the refresh for 100 simulated seconds: far more ticks than a
    # socket buffer holds, so the broker's own queue is what is tested
    b.feed("F 6000000 %d %d" % (P60, P60 // 1000))
    st = b.stats(timeout=60)
    check(st["on"] == 1, "the broker kept running (it answered the stats barrier)")
    check(st["tx_used"] <= 3, "the queue holds at most 3 packets (got %d)" % st["tx_used"])
    check(st["coalesced"] - base["coalesced"] > 0, "ticks were coalesced in the queue (%d)" % (st["coalesced"] - base["coalesced"]))
    check(st["ticks"] - base["ticks"] > 8000, "enough ticks to fill the socket buffer (%d)" % (st["ticks"] - base["ticks"]))
    print("   never-reads: %d ticks emitted, %d coalesced in the queue, queue %d, %d dropped too fast"
          % (st["ticks"] - base["ticks"], st["coalesced"] - base["coalesced"], st["tx_used"], st["fast"] - base["fast"]))
    # now read it all: well-formed, increasing, and the live request is there
    raws = read_raw(s, 2.0)
    while True:
        more = read_raw(s, 0.3)
        if not more:
            break
        raws += more
    t = ticks_of(raws)
    check(len(t) > 0 and all(a.tick_seq < b_.tick_seq for a, b_ in zip(t, t[1:])),
          "what the peer reads afterwards is well formed and increasing (%d ticks)" % len(t))
    check(any(x.want for x in t), "and the live request is in it")
    s.close()
    time.sleep(0.2)


def hostile_replies(b):
    print("-- a VMM flooding replies is cut off by the rate limit; the broker serves on")
    t0 = vt(b)
    s, _ = connect(b, height=75)
    read_raw(s, 0.2)
    b.feed("t %d %d" % (t0 + P60, P60), "t %d %d" % (t0 + 2 * P60, P60))
    read_raw(s, 0.2)
    base = b.stats()
    for _ in range(5):
        s.sendall(b"".join(nochange(2) for _ in range(500)))
    time.sleep(0.3)
    st = b.stats()
    check(st["late"] - base["late"] == 2500, "2500 late replies: counted, connection lives (%d)" % (st["late"] - base["late"]))
    check(not tc.closed(s, 0.2), "the connection lives")
    try:
        for _ in range(100):
            s.sendall(b"".join(nochange(2) for _ in range(500)))
    except OSError:
        pass
    check(violated(s), "50000 replies in a burst: disconnected by the command rate limit")
    check(alive(b), "and the broker serves the next client")
    s.close()


def reconnect(b):
    print("-- tick state is connection state")
    t0 = vt(b)
    s, _ = connect(b, height=75)
    read_raw(s, 0.2)
    b.feed("t %d %d" % (t0 + P60, P60))
    check(len(ticks_of(read_raw(s, 0.3))) == 1, "ticks flow")
    s.close()
    time.sleep(0.3)
    s2, _ = connect(b, opt_in=False)
    b.feed("t %d %d" % (t0 + 2 * P60, P60))
    check(ticks_of(read_raw(s2, 0.3)) == [], "the next connection, not opted in, gets none")
    check(b.stats()["on"] == 0, "and the state is off")
    s2.sendall(caps(CLIENT_TICK, 75))
    t = ticks_of(read_raw(s2, 0.3))
    check(len(t) == 1 and t[0].tick_seq == 1 and t[0].want,
          "opting in again starts from tick 1 with a request")
    s2.close()


def old_broker(tmp):
    print("-- a broker that predates ticks (CAP_TICK clear): a VMM must not opt in")
    os.environ["NB_TEST_NO_TICK"] = "1"
    try:
        b = B(os.path.join(tmp, "old.sock"))
    finally:
        del os.environ["NB_TEST_NO_TICK"]
    try:
        s, hs = connect(b, opt_in=False)
        check(not (hs[0][6] & CAP_TICK), "HELLO does not advertise CAP_TICK")
        s.sendall(caps(0, 0))
        b.feed("t %d %d" % (2 * 10**9, P60))
        check(ticks_of(read_raw(s, 0.3)) == [], "no ticks")
        check(not tc.closed(s, 0.2), "CAPS(0) is fine")
        s.sendall(caps(CLIENT_TICK, 60))
        check(violated(s), "CLIENT_TICK on a broker that did not advertise it is a violation")
        s.close()
    finally:
        b.stop()


if __name__ == "__main__":
    main()
