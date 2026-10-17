#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""The tick pump against a REAL compositor: mutter --headless (software
renderer, a virtual 60 Hz monitor), the broker's Wayland backend, the real
socket.  Skips (exit 77) when mutter or dbus-run-session is missing.

Asserted, MEASURED not modelled:
  * after the VMM opts in, EV_TICK flows at the compositor's refresh rate,
    with the compositor's own presentation timestamps (not CLOCK_ONLY on
    mutter, whose presentation clock is CLOCK_MONOTONIC) and a period of
    one refresh;
  * the lock step: a frame answering each WANT_FRAME request (COMMIT.seq =
    request) is followed by the next request only after it was presented;
    answering with NO_CHANGE opens the next one at the next tick;
  * with nobody answering, ticks still flow and a new request opens only
    after the 100 ms timeout;
  * when the compositor stops presenting (SIGSTOP), the ticks STOP -- the
    VMM's 50 ms fallback is what takes over -- and resume by themselves
    after SIGCONT (the pump's watchdog);
and, as a deliberate gap in the other direction, nothing here claims anything
about a real GNOME session (see docs/design/V3_DISPLAY_PACING_20261017.md
section 11.8).
"""
import os
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_cursor as tc   # noqa: E402
import test_tick as tt     # noqa: E402
import test_xor as tx      # noqa: E402

check = tc.check


def skip(why):
    print("SKIP: " + why)
    sys.exit(77)


class Session:
    def __init__(self, broker, tmp):
        self.broker = broker
        self.xdg = os.path.join(tmp, "xdg")
        os.makedirs(self.xdg, mode=0o700, exist_ok=True)
        self.log_path = os.path.join(tmp, "broker.log")

    def start_compositor(self):
        env = dict(os.environ, XDG_RUNTIME_DIR=self.xdg, GIO_USE_VFS="local")
        self.mutter = subprocess.Popen(
            ["dbus-run-session", "--", "mutter", "--headless", "--wayland",
             "--no-x11", "--wayland-display", "wl-tick", "--virtual-monitor",
             "1024x768"], env=env, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, start_new_session=True)
        for _ in range(100):
            if os.path.exists(os.path.join(self.xdg, "wl-tick")):
                return True
            time.sleep(0.1)
        return False

    def mutter_pids(self):
        out = subprocess.run(["pgrep", "-g", str(self.mutter.pid), "-x", "mutter"],
                             capture_output=True, text=True).stdout.split()
        return [int(x) for x in out]

    def stop_compositor(self):
        try:
            os.killpg(self.mutter.pid, signal.SIGCONT)
            os.killpg(self.mutter.pid, signal.SIGTERM)
        except OSError:
            pass

    def start_broker(self):
        env = dict(os.environ, XDG_RUNTIME_DIR=self.xdg, WAYLAND_DISPLAY="wl-tick",
                   ASAN_OPTIONS="detect_leaks=0")
        self.sock = os.path.join(self.xdg, "b.sock")
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


def collect(s, seconds, answer=None, idle=None):
    """Read for `seconds`, returning [(t_mono, Tick)]; `answer(tick)` is called
    for each WANT_FRAME tick as it arrives, `idle()` on every pass."""
    out = []
    end = time.monotonic() + seconds
    buf = b""
    s.settimeout(0.01)
    while time.monotonic() < end:
        try:
            part = s.recv(65536)
        except socket.timeout:
            if idle:
                idle()
            continue
        except OSError:
            break
        if not part:
            break
        if idle:
            idle()
        now = time.monotonic()
        buf += part
        while len(buf) >= 24:
            raw, buf = buf[:24], buf[24:]
            if tt.struct.unpack_from("<H", raw)[0] == tt.EV_TICK:
                t = tt.Tick(raw)
                out.append((now, t))
                if answer and t.want:
                    answer(t)
    return out


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: test_tickpump.py /path/to/broker")
    broker = os.path.abspath(sys.argv[1])
    for tool in ("mutter", "dbus-run-session", "pgrep"):
        if not shutil.which(tool):
            skip(tool + " is not installed")
    with tempfile.TemporaryDirectory(prefix="nbtick", ignore_cleanup_errors=True) as tmp:
        sess = Session(broker, tmp)
        if not sess.start_compositor():
            sess.stop_compositor()
            skip("mutter --headless did not come up")
        try:
            sess.start_broker()
            time.sleep(0.6)
            log = sess.broker_log()
            if "display ticks:" not in log:
                skip("this broker build has no Wayland backend")
            run(sess)
        finally:
            try:
                sess.stop_broker()
            except Exception:
                pass
            sess.stop_compositor()
    if tc.FAILS:
        print("%d tick-pump check(s) FAILED" % len(tc.FAILS))
        sys.exit(1)
    print("tick pump tests passed")


def run(sess):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(sess.sock)
    hs = [tc.PKT.unpack(tc.recv_exact(s, tc.PKT.size)) for _ in range(6)]
    check(bool(hs[0][6] & tt.CAP_TICK), "the Wayland backend advertises CAP_TICK")
    s.sendall(tt.caps(tt.CLIENT_TICK, 75))

    print("-- nobody answers: ticks at the refresh rate, one request per 100 ms")
    got = collect(s, 3.0)
    ticks = [t for _, t in got]
    check(len(ticks) >= 3 * 50, "at least 50 ticks/s from the compositor (got %d in 3 s)" % len(ticks))
    check(len(ticks) <= 3 * 70, "and not more than 70/s (got %d)" % len(ticks))
    ivs = [b - a for (a, _), (b, _) in zip(got[10:], got[11:])]
    if ivs:
        mean = statistics.mean(ivs) * 1000
        check(14.0 <= mean <= 20.0, "mean tick spacing %.2f ms is about one refresh (16.7)" % mean)
    pres = [b.present - a.present for a, b in zip(ticks[10:], ticks[11:])]
    if pres:
        m = statistics.mean(pres) / 1e6
        check(15.5 <= m <= 17.8, "the compositor's own timestamps are %.3f ms apart" % m)
    check(all(not t.clock_only for t in ticks[3:]), "mutter's clock is CLOCK_MONOTONIC: not CLOCK_ONLY")
    check(all(a.tick_seq + 1 == b.tick_seq for a, b in zip(ticks, ticks[1:])), "tick_seq has no gap")
    check(all(a.present <= b.present for a, b in zip(ticks, ticks[1:])), "present_ns never decreases")
    per = [t for t in ticks if t.period]
    check(per and all(15_000_000 <= t.period <= 18_000_000 for t in per[3:]), "period words are one refresh")
    wants = [(tm, t) for tm, t in got if t.want]
    check(len(wants) <= 1 + int(3.0 / 0.1) + 1, "no more than one request per 100 ms (got %d)" % len(wants))
    check(len(wants) >= 20, "and a request still opens every 100 ms (got %d)" % len(wants))

    print("-- lock step: a frame per request, the next request after its presentation")
    fds = []

    def answer_frame(t):
        tt.frame(s, t.tick_seq)

    got = collect(s, 3.0, answer_frame)
    ticks = [t for _, t in got]
    wants = [t for t in ticks if t.want]
    check(len(wants) >= 0.7 * len(ticks),
          "a VMM that answers at once gets a request on most ticks: %d of %d" % (len(wants), len(ticks)))
    check(all(a.tick_seq < b.tick_seq for a, b in zip(wants, wants[1:])), "request numbers increase")

    print("-- a slow VMM (answers 40 ms late): the next request waits for the answer AND the presentation")
    late = []

    def slow_answer(t):
        late.append((time.monotonic() + 0.040, t.tick_seq))

    def flush_late():
        now = time.monotonic()
        while late and late[0][0] <= now:
            tt.frame(s, late.pop(0)[1])

    got = collect(s, 3.0, slow_answer, flush_late)
    ticks = [t for _, t in got]
    wants = [t for t in ticks if t.want]
    gaps = [b.tick_seq - a.tick_seq for a, b in zip(wants, wants[1:])]
    check(gaps and min(gaps) >= 3,
          "requests are at least 3 ticks apart (answer 40 ms = 2.4 ticks, then a presentation): min %r"
          % (min(gaps) if gaps else None))
    check(gaps and max(gaps) <= 6, "and no further apart than that plus a tick or two: max %r" % (max(gaps) if gaps else None))
    check(len(wants) >= 20, "(%d requests in 3 s)" % len(wants))
    late.clear()
    log = sess.broker_log()
    check("ticks: ON" in log, "the log says ticks are ON, once")
    check(log.count("ticks: ON") == 1, "exactly one tick-mode line")

    print("-- NO_CHANGE: the next request on the next tick")
    got = collect(s, 2.0, lambda t: s.sendall(tt.nochange(t.tick_seq)))
    ticks = [t for _, t in got]
    wants = [t for t in ticks if t.want]
    check(len(wants) >= len(ticks) // 3, "a request every other tick or so (%d of %d)" % (len(wants), len(ticks)))
    gaps = [b.tick_seq - a.tick_seq for a, b in zip(wants, wants[1:])]
    check(gaps and max(gaps) <= 3, "never more than 3 ticks between requests (%r)" % (max(gaps) if gaps else None))

    print("-- the compositor stops presenting: ticks stop, then resume by themselves")
    pids = sess.mutter_pids()
    check(bool(pids), "found the compositor to stop")
    for p in pids:
        os.kill(p, signal.SIGSTOP)
    collect(s, 0.6)                   # whatever was already in flight
    got = collect(s, 1.5)
    check(len(got) <= 2, "no ticks while the compositor is stopped (got %d in 1.5 s, after 0.6 s of settling)" % len(got))
    for p in pids:
        os.kill(p, signal.SIGCONT)
    t_cont = time.monotonic()
    got = collect(s, 1.5)
    check(len(got) >= 40, "ticks resume after SIGCONT (%d in 1.5 s)" % len(got))
    if got:
        check(got[0][0] - t_cont < 0.8, "within 0.8 s (the watchdog is 250 ms): %.2f s" % (got[0][0] - t_cont))
    s.close()


if __name__ == "__main__":
    main()
