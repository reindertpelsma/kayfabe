#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The display broker in the CI box run (docs/design/V3_BROKER_IN_CI.md).

STATUS: DESIGN/PLUMBING, 2026-11-12. GPU-free tests only (scripts/ci/test_broker_ci.py); NOTHING here has run on a box.

Owner ruling (2026-11-12): a failing broker is a failing kayfabe. Every class below is a FAIL attributed to kayfabe.
None of them is ever a retry, operator, provider, unattributed or not-run class, and none is excused. The only
exception lives in vast_payload.broker_guarded: a ProviderFault raised by a stage keeps its own provider verdict.

Two entry points, both with the logic in Python and only shell glue around them:
  broker_ci.py run --name NAME [--dir DIR] -- COMMAND...    one broker (the headless `test` backend, --persist) for
        one stage: started before the command, stopped after it, its counters printed as ONE `BROKER_SUMMARY` line.
  broker_ci.py explicit --tag TAG --prov DIR                the short explicit stage (Windows guest): first frame
        not blank, one resolution change reaches the broker as a new attach, one key and absolute pointer event
        reaches an in-guest logger. Prints ONE `BROKER_EXPLICIT` line.
The broker is a separate process in its own session with PDEATHSIG, so it is never in a vCPU thread or a lock a
vCPU takes; the harness talks to it through a stdin FIFO opened O_NONBLOCK (a write that would block fails).
"""

import ctypes
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

# Row names are limited to [a-z][a-z0-9_]{0,47} (vast_report.SUITE_NAME), so the rows are broker_frames, not broker:frames.
CLASSES = (
    "broker_build",       # the broker binary could not be built or is missing
    "broker_start",       # it did not start, no socket, or the stage's wrapper left no summary
    "broker_died",        # it exited while the guest ran
    "broker_error",       # its log shows a framing violation / crash marker
    "broker_no_attach",   # the guest never connected or never attached a frame buffer
    "broker_no_frames",   # attached, but not one frame was committed
    "broker_late_attach", # the first attach came later than ATTACH_LIMIT_S after the guest command started
    "broker_leftover",    # it was still alive after the stop
    "broker_frames",      # explicit stage: no first frame or a blank one
    "broker_resize",      # explicit stage: the resolution change did not arrive as a new attach
    "broker_input",       # explicit stage: the scripted key / absolute pointer event did not reach the guest
)
# Stage-owned refusals that happen BEFORE a guest could start; with a healthy broker that saw nothing they keep their class
# (vast_payload.broker_guarded). Never anything else.
PREBOOT_SUFFIXES = ("_retry_media", "_retry_time", "_retry_no_list", "_retry_refused")

START_WAIT_S = 15
STOP_WAIT_S = 10
ATTACH_LIMIT_S = 900  # UNMEASURED guess: a Windows boot to desktop took ~40 s on the PC, a fat Linux boot is longer
SOCKET_ROOT = "/run/kf-broker"


class BrokerFailure(Exception):
    """A broker failure. `name` is one of CLASSES; rows are the rows already earned."""

    def __init__(self, name, rows=()):
        assert name in CLASSES, name
        self.name = name
        self.rows = list(rows)
        super().__init__(name)


def row(name, passed=0, failed=0, skipped=0):
    return {"name": name, "pass": passed, "fail": failed, "skip": skipped}


def broker_wanted(environ, tier):
    """Default ON for middle and large, OFF for small/adhoc/unknown; KF_BROKER=0|1 overrides (manual runs).
    Returns (wanted, source). A malformed KF_BROKER raises ValueError (the payload turns it into preflight)."""
    value = environ.pop("KF_BROKER", None)
    if value is not None:
        if value not in ("0", "1"):
            raise ValueError("KF_BROKER")
        return value == "1", "env"
    return tier in ("middle", "large"), "tier"


def kf3_props(sock, uid=0):
    """The kf3-gpu properties that attach a device to the broker (Linux: appended to KF3_DEV_EXTRA)."""
    return "display=on,display-broker=" + str(sock) + ",display-broker-uid=" + str(int(uid))


def child_environment(environ, sock):
    env = dict(environ)
    existing = env.get("KF3_DEV_EXTRA", "")
    env["KF3_DEV_EXTRA"] = (existing + "," if existing else "") + kf3_props(sock)
    env["WINVM_BROKER_SOCK"] = str(sock)  # win_vm.sh adds display-broker + qemu-xhci/usb-tablet when set
    env["KF_BROKER_SOCK"] = str(sock)
    return env


# ---------------------------------------------------------------- the broker's log

ACCEPT = re.compile(r"accepted uid \d+", re.M)
ATTACH = re.compile(r"TEST attach: id=\d+ (\d+)x(\d+) ", re.M)
COMMIT = re.compile(r"TEST commit: id=\d+ \d+x\d+", re.M)
# GUESSED markers (not measured): a violation the broker names, or a crash.
ERROR = re.compile(r"framing violation|Segmentation fault|AddressSanitizer|\bFATAL\b|\bfatal:", re.I | re.M)


def parse_log(text):
    sizes = [w + "x" + h for w, h in ATTACH.findall(text)]
    return {
        "accepted": len(ACCEPT.findall(text)),
        "attaches": len(sizes),
        "commits": len(COMMIT.findall(text)),
        "sizes": sizes,
        "errors": len(ERROR.findall(text)),
    }


def count_attach(text, size):
    return sum(1 for s in parse_log(text)["sizes"] if s == size)


# ---------------------------------------------------------------- the summary line

SUMMARY = re.compile(
    r"^BROKER_SUMMARY name=([a-z][a-z0-9_]{0,31}) started=([01]) built=([01]) accepted=([0-9]+) attaches=([0-9]+)"
    r" commits=([0-9]+) first_attach_s=(-|[0-9]+) died=([01]) errors=([0-9]+) stopped=([01])$",
    re.M,
)


def format_summary(name, d):
    first = "-" if d.get("first_attach_s") is None else str(int(d["first_attach_s"]))
    return (
        "BROKER_SUMMARY name=%s started=%d built=%d accepted=%d attaches=%d commits=%d first_attach_s=%s died=%d"
        " errors=%d stopped=%d"
        % (name, d["started"], d["built"], d["accepted"], d["attaches"], d["commits"], first, d["died"],
           d["errors"], d["stopped"])
    )


def parse_summary(text, name):
    """The one summary of `name`, or None (none, or two: a log that cannot be trusted)."""
    found = [m for m in SUMMARY.findall(text) if m[0] == name]
    if len(found) != 1:
        return None
    m = found[0]
    return {
        "started": int(m[1]), "built": int(m[2]), "accepted": int(m[3]), "attaches": int(m[4]),
        "commits": int(m[5]), "first_attach_s": None if m[6] == "-" else int(m[6]), "died": int(m[7]),
        "errors": int(m[8]), "stopped": int(m[9]),
    }


def failure_class(d):
    """The BROKER_* class of a summary, or None when the broker did its job. Every class is a FAIL."""
    if d is None:
        return "broker_start"
    if not d["built"]:
        return "broker_build"
    if not d["started"]:
        return "broker_start"
    if d["died"]:
        return "broker_died"
    if d["errors"]:
        return "broker_error"
    if d["accepted"] == 0 or d["attaches"] == 0:
        return "broker_no_attach"
    if d["commits"] == 0:
        return "broker_no_frames"
    if d["first_attach_s"] is not None and d["first_attach_s"] > ATTACH_LIMIT_S:
        return "broker_late_attach"
    if not d["stopped"]:
        return "broker_leftover"
    return None


def stage_rows(text, name):
    """The rows of a broker-attached stage: [broker_<name> pass] or BrokerFailure. `name` is the stage name."""
    cls = failure_class(parse_summary(text, name))
    if cls is not None:
        raise BrokerFailure(cls)
    return [row(("broker_" + name)[:48], 1)]


# ---------------------------------------------------------------- the lifecycle

def _pdeathsig():
    # the broker dies with the wrapper, even on SIGKILL (Linux prctl PR_SET_PDEATHSIG = 1)
    try:
        ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, int(signal.SIGKILL))
    except OSError:
        pass


class BrokerSession:
    """One persistent test-backend broker. `spawn` and `clock` are injectable for tests."""

    def __init__(self, binary, directory, name, spawn=subprocess.Popen, clock=time.monotonic, sleep=time.sleep,
                 size="1920x1080"):
        self.binary, self.name, self.size = str(binary), name, size
        self.dir = Path(directory)
        self.sock = self.dir / (name + ".sock")
        self.log = self.dir / (name + ".log")
        self.fifo = self.dir / (name + ".in")
        self.spawn, self.clock, self.sleep = spawn, clock, sleep
        self.proc = None
        self.fifo_fd = None
        self.logfile = None
        self.started = 0
        self.died = 0
        self.stopped = 0
        self.t0 = None
        self.first_attach = None

    def start(self, wait=None):
        wait = START_WAIT_S if wait is None else wait
        self.dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(self.dir, 0o700)
        for p in (self.sock, self.fifo):
            p.unlink(missing_ok=True)
        os.mkfifo(self.fifo, 0o600)
        # Two open file descriptions of the FIFO (O_NONBLOCK is per description): the broker's own stdin, blocking
        # as a shell redirect would be (O_RDWR, so it never sees EOF before we stop it), and ours, non-blocking,
        # which only writes. The broker sets its own stdin non-blocking if it wants to.
        child_in = os.open(self.fifo, os.O_RDWR)
        self.fifo_fd = os.open(self.fifo, os.O_RDWR | os.O_NONBLOCK)
        self.logfile = open(self.log, "wb")
        argv = [self.binary, "--socket", str(self.sock), "--backend", "test", "--persist", "--verbose",
                "--size", self.size]
        try:
            self.proc = self.spawn(argv, stdin=child_in, stdout=self.logfile, stderr=subprocess.STDOUT,
                                   start_new_session=True, preexec_fn=_pdeathsig)
        except OSError:
            return False
        finally:
            os.close(child_in)
        end = self.clock() + wait
        while self.clock() < end:
            if self.proc.poll() is not None:
                return False
            if self.sock.exists():
                self.started = 1
                return True
            self.sleep(0.2)
        return False

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def mark_guest_started(self):
        self.t0 = self.clock()

    def log_text(self):
        try:
            return self.log.read_bytes().decode("utf-8", errors="replace")
        except OSError:
            return ""

    def poll(self):
        """Called by the wrapper's loop: notes a death and the time of the first attach."""
        if self.started and not self.alive() and not self.stopped:
            self.died = 1
        if self.first_attach is None and self.t0 is not None and parse_log(self.log_text())["attaches"]:
            self.first_attach = self.clock() - self.t0

    def send(self, line):
        """One test-backend stdin line (docs: f/p/a/k/b/r/w). Never blocks: a full FIFO is an error."""
        os.write(self.fifo_fd, (line.rstrip("\n") + "\n").encode())

    def stop(self):
        self.poll()
        if self.proc is not None and self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            end = self.clock() + STOP_WAIT_S
            while self.proc.poll() is None and self.clock() < end:
                self.sleep(0.1)
            if self.proc.poll() is None:
                try:
                    os.killpg(self.proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                end = self.clock() + STOP_WAIT_S
                while self.proc.poll() is None and self.clock() < end:
                    self.sleep(0.1)
        self.stopped = int(self.proc is None or self.proc.poll() is not None)
        for fd in (self.fifo_fd,):
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass
        self.fifo_fd = None
        if self.logfile:
            self.logfile.close()
        self.sock.unlink(missing_ok=True)
        self.fifo.unlink(missing_ok=True)

    def summary(self, built=1):
        d = parse_log(self.log_text())
        return {
            "started": self.started, "built": built, "accepted": d["accepted"], "attaches": d["attaches"],
            "commits": d["commits"], "first_attach_s": self.first_attach, "died": self.died,
            "errors": d["errors"], "stopped": self.stopped,
        }


def ensure_binary(root, runner=subprocess.run):
    """The broker binary (broker/nvkvm-display-broker), built with make when absent. Returns (path, built_ok)."""
    binary = Path(root) / "broker" / "nvkvm-display-broker"
    if binary.is_file() and os.access(binary, os.X_OK):
        return binary, 1
    try:
        runner(["make", "-C", str(Path(root) / "broker")], check=True, timeout=600,
               stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    except (OSError, subprocess.SubprocessError):
        return binary, 0
    return binary, int(binary.is_file() and os.access(binary, os.X_OK))


def run_stage(name, command, root, directory=SOCKET_ROOT, environ=None, session_factory=BrokerSession,
              binary_finder=ensure_binary, popen=subprocess.Popen, out=sys.stdout, tick=2.0, sleep=time.sleep):
    """`broker_ci.py run`: the broker around ONE stage command. Returns the command's exit code (1 when the
    broker never started: the command does not run without its broker)."""
    environ = os.environ if environ is None else environ
    binary, built = binary_finder(root)
    session = session_factory(binary, directory, name)
    if not built:
        print(format_summary(name, dict(started=0, built=0, accepted=0, attaches=0, commits=0, first_attach_s=None,
                                        died=0, errors=0, stopped=1)), file=out, flush=True)
        return 1
    if not session.start():
        session.stop()
        print(format_summary(name, session.summary()), file=out, flush=True)
        return 1
    stop = []
    old = signal.signal(signal.SIGTERM, lambda *_: stop.append(1))
    child = None
    rc = 1
    try:
        session.mark_guest_started()
        child = popen(command, env=child_environment(environ, session.sock))
        while child.poll() is None:
            session.poll()
            if stop:
                # the payload's group SIGTERM reached the child too; give it its time, then kill it
                try:
                    child.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    child.kill()
            elif tick:
                sleep(tick)
        rc = child.wait()
    finally:
        signal.signal(signal.SIGTERM, old)
        if child is not None and child.poll() is None:
            child.kill()
            child.wait()
        session.stop()
        print(format_summary(name, session.summary()), file=out, flush=True)
    return rc


# ---------------------------------------------------------------- the explicit stage

BLANK_SD = 0.01          # a frame whose pixel standard deviation is below this is blank (the PC desktop: 0.25)
INPUT_TOLERANCE_PX = 2   # PC measurement: absolute events arrived 1 px low
POINTS = ((960, 540), (1800, 1000))
KEY_EVDEV, KEY_VK = 30, 0x41  # KEY_A -> VK_A
SCRIPT_LINES = ["f 1", "p 1", "a %d %d" % POINTS[0], "a %d %d" % POINTS[1], "k %d 1" % KEY_EVDEV,
                "k %d 0" % KEY_EVDEV]

KFIN_MOVE = re.compile(r"^KFIN \d+ M msg=0x200 x=(-?\d+) y=(-?\d+) ", re.M)
KFIN_KEY = re.compile(r"^KFIN \d+ K vk=0x([0-9a-f]{2}) sc=0x[0-9a-f]{2} (down|up) ", re.M)


def pixels_blank(mean, sd):
    return sd < BLANK_SD


def input_arrived(logger_text):
    """Both absolute points (within the tolerance) and the key down then up reached the in-guest logger."""
    moves = [(int(x), int(y)) for x, y in KFIN_MOVE.findall(logger_text)]
    for px, py in POINTS:
        if not any(abs(x - px) <= INPUT_TOLERANCE_PX and abs(y - py) <= INPUT_TOLERANCE_PX for x, y in moves):
            return False
    dirs = [d for v, d in KFIN_KEY.findall(logger_text) if int(v, 16) == KEY_VK]
    if "down" not in dirs:
        return False
    return "up" in dirs[dirs.index("down"):]


def wait_for(predicate, timeout, clock=time.monotonic, sleep=time.sleep, step=0.5):
    end = clock() + timeout
    while True:
        if predicate():
            return True
        if clock() >= end:
            return False
        sleep(step)


def run_explicit(session, guest, clock=time.monotonic, sleep=time.sleep, frame_wait=90, resize_wait=45,
                 input_wait=20):
    """Returns the rows; raises BrokerFailure (rows = what passed before) on the first failing step. `guest` offers
    screendump_stats() -> (mean, sd), set_resolution(w, h), logger_start(), logger_text()."""
    rows = []
    # frames: the broker's own commit counter, then a screendump of the guest head that is not blank
    if not wait_for(lambda: parse_log(session.log_text())["commits"] >= 1, frame_wait, clock, sleep):
        raise BrokerFailure("broker_frames", rows)
    mean, sd = guest.screendump_stats()
    if pixels_blank(mean, sd):
        raise BrokerFailure("broker_frames", rows)
    rows.append(row("broker_frames", 1))
    # resize: a guest mode change must arrive at the broker as a NEW attach of the new size
    before = count_attach(session.log_text(), "1280x720")
    guest.set_resolution(1280, 720)
    arrived = wait_for(lambda: count_attach(session.log_text(), "1280x720") > before, resize_wait, clock, sleep)
    back_before = count_attach(session.log_text(), "1920x1080")
    guest.set_resolution(1920, 1080)
    restored = wait_for(lambda: count_attach(session.log_text(), "1920x1080") > back_before, resize_wait, clock, sleep)
    if not (arrived and restored):
        raise BrokerFailure("broker_resize", rows)
    rows.append(row("broker_resize", 1))
    # input: scripted lines on the broker's stdin; the guest's low-level hook logger sees them
    guest.logger_start()
    for line in SCRIPT_LINES:
        session.send(line)
        sleep(0.25)
    if not wait_for(lambda: input_arrived(guest.logger_text()), input_wait, clock, sleep):
        raise BrokerFailure("broker_input", rows)
    rows.append(row("broker_input", 1))
    return rows


EXPLICIT = re.compile(
    r"^BROKER_EXPLICIT frames=(PASS|FAIL) resize=(PASS|FAIL|NOT_RUN) input=(PASS|FAIL|NOT_RUN)(?: class=(broker_[a-z_]+))?$",
    re.M,
)


def format_explicit(rows, failure=None):
    got = {r["name"]: r["pass"] for r in rows}
    state = {}
    failed = failure.name if failure else None
    for step in ("broker_frames", "broker_resize", "broker_input"):
        state[step] = "PASS" if got.get(step) else ("FAIL" if step == failed else "NOT_RUN")
    if failed is None and not all(v == "PASS" for v in state.values()):
        state["broker_frames"] = "FAIL"
    if state["broker_frames"] == "NOT_RUN":
        state["broker_frames"] = "FAIL"
    line = "BROKER_EXPLICIT frames=%s resize=%s input=%s" % (
        state["broker_frames"], state["broker_resize"], state["broker_input"])
    if failure is not None and failure.name not in state:
        line += " class=" + failure.name  # broker_start / broker_build: before any step ran
    return line


def explicit_rows(text):
    """Grade the explicit stage output. A missing or duplicated line is broker_start, NOT a retry: no verdict is a FAIL."""
    found = EXPLICIT.findall(text)
    if len(found) != 1:
        raise BrokerFailure("broker_start")
    frames, resize, inp, cls = found[0]
    if cls and cls in CLASSES and (frames, resize, inp) == ("FAIL", "NOT_RUN", "NOT_RUN"):
        raise BrokerFailure(cls)
    rows = []
    for step, verdict in (("broker_frames", frames), ("broker_resize", resize), ("broker_input", inp)):
        if verdict != "PASS":
            raise BrokerFailure(step, rows)
        rows.append(row(step, 1))
    return rows


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    root = Path(__file__).resolve().parents[2]
    if argv[:1] == ["run"] and "--" in argv:
        cut = argv.index("--")
        opts, command = argv[1:cut], argv[cut + 1:]
        name = opts[opts.index("--name") + 1] if "--name" in opts else ""
        directory = opts[opts.index("--dir") + 1] if "--dir" in opts else SOCKET_ROOT
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,31}", name) or not command:
            return 2
        return run_stage(name, command, root, directory)
    if argv[:1] == ["explicit"]:
        from broker_explicit_guest import explicit_main  # the Windows guest driver (shell-out glue)
        return explicit_main(argv[1:], root)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
