#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The time budget and the progress of the copy into the guest (stdlib only, GPU-free).

STATUS: CODE, 2026-11-20; GPU-free tests only (scripts/ci/test_spec_copy_budget.py, scripts/ci/test_vast_spec_stage.py,
mutants in scripts/ci/spec_stage_mutants.py). Used by scripts/bench/box/specviewperf_stage.sh; docs/design/V3_SPEC_GATE.md.

Why it exists (MEASURED 2026-11-20): the copy of a lane's tree into the Windows guest
(`stream | zstd -d | tarfilter.py | ssh ... tar -xf -`) ran at about 5 MB/s on one Vast box and hit its fixed 70-minute budget; the
stage ended `RETRY class=TIME` with every row NOT_RUN SETUP and nothing in its log about the rate or the bytes that had arrived.

    copy_budget.py check [--budget V] [--note-s V]        refuse a bad KF_SPEC_COPY_BUDGET_S / KF_SPEC_COPY_NOTE_S by name (status 2)
    copy_budget.py budget --lane L --knob V --left S      print `<budget_s> <derived|knob> <expected_bytes> <unclamped_s> <why>`
    copy_budget.py pipe --status F --log L --budget B --expected N [--note-s V | --interval S]
                                                          stdin to stdout, counting; a progress note every 60 s (KF_SPEC_COPY_NOTE_S)
    copy_budget.py abort-text --status F --budget B --expected N     the sentence of the TIME abort (bytes, rate, needed_s)

The budget is DERIVED, never typed: the lane's size (lanes.json `tree_gb`, the table the stage already trusts) at a floor rate, plus a
margin, never above the time left; the knob KF_SPEC_COPY_BUDGET_S (600 to 86400 s) replaces the derived number and is clamped
to the time left too. `pipe` sits between tarfilter.py and the ssh tar, so its count is what was handed to the guest's tar.
It is advisory: a log or status file it cannot write never fails the copy.
"""

import argparse
import math
import os
import re
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lane as lane_mod  # noqa: E402

KNOB = "KF_SPEC_COPY_BUDGET_S"
NOTE_KNOB = "KF_SPEC_COPY_NOTE_S"
KNOB_MIN_S = 600
KNOB_MAX_S = 86400
NOTE_DEFAULT_S = 60
NOTE_MIN_S = 1
NOTE_MAX_S = 3600

# The floor rate of the copy. MEASURED 2026-11-20, on one box only (a Vast RTX 4090 offer, an EPYC 7B13 VM, tree 7fb1c98a0): the stream ran
# at about 5 MB/s (its cause is INFERRED: one host thread bound by the guest's user-mode network). The earlier box (a Ryzen 9 5950X)
# did the same step at 13.7 MB/s (51 minutes); the guest disk itself takes 40 to 45 MB/s (2026-11-05). One box is not a law: a
# slower host needs KF_SPEC_COPY_BUDGET_S, and the `copy progress` lines of the stage log say what rate a run really had.
FLOOR_RATE_BYTES_S = 5_000_000
MARGIN_PCT = 20  # on top of the time at the floor rate (INFERRED, a choice)
FIXED_S = KNOB_MIN_S  # the ssh start-up, the guest's tar finishing and the stream's last bytes (INFERRED, a choice)
GIB = 1 << 30  # tree_gb is read as GiB: a budget is a ceiling, so the larger reading
CHUNK = 65536
STATUS_RE = re.compile(r"^COPY_PROGRESS bytes=([0-9]+) elapsed_s=([0-9]+(?:\.[0-9]+)?) final=([01])$")


class KnobError(ValueError):
    pass


def _whole(text, name, lo, hi, what):
    if text is None or text == "":
        return None
    if re.fullmatch(r"[0-9]{1,9}", text, re.ASCII) is None or not lo <= int(text) <= hi:
        raise KnobError("%s must be %s from %d to %d (got %s)" % (name, what, lo, hi, ascii(text[:24])))
    return int(text)  # decimal: a leading zero is not octal here


def parse_knob(text):
    """KF_SPEC_COPY_BUDGET_S: None when unset or empty (the derived default), else whole seconds from 600 to 86400."""
    return _whole(text, KNOB, KNOB_MIN_S, KNOB_MAX_S, "a whole number of seconds")


def parse_note_s(text):
    """KF_SPEC_COPY_NOTE_S: the seconds between two progress lines (default 60), 1 to 3600."""
    got = _whole(text, NOTE_KNOB, NOTE_MIN_S, NOTE_MAX_S, "a whole number of seconds")
    return NOTE_DEFAULT_S if got is None else got


def expected_bytes(lane, tbl=None):
    """What the copy moves: the trees of the lane's viewsets (the lane table; the CLI and the runtime are small beside them)."""
    tbl = tbl or lane_mod.table()
    return sum(int(round(tbl["viewsets"][w]["tree_gb"] * GIB)) for w in lane_mod.lane_ids(lane, tbl))


def derived_budget_s(nbytes):
    """The size at the floor rate, plus the margin, plus the fixed allowance (which is also the knob's lower bound: no copy gets less)."""
    at_floor = -(-int(nbytes) * (100 + MARGIN_PCT) // (FLOOR_RATE_BYTES_S * 100))
    return at_floor + FIXED_S


def budget(lane, knob_text, left_s, tbl=None):
    """(budget_s, source, expected_bytes, unclamped_s): the knob if set, else the derived number, then clamped to the time left
    (at least one second: `timeout 0` would mean no timeout at all)."""
    knob = parse_knob(knob_text)
    nbytes = expected_bytes(lane, tbl)
    if knob is not None:
        unclamped, source = knob, "knob"
    else:
        unclamped, source = derived_budget_s(nbytes), "derived"
    return min(unclamped, max(int(left_s), 1)), source, nbytes, unclamped


def rate_mb_s(nbytes, elapsed_s):
    """Decimal megabytes per second; zero when nothing moved or no time has gone."""
    if nbytes <= 0 or elapsed_s <= 0:
        return 0.0
    return round(nbytes / elapsed_s / 1e6, 1)


def needed_s(expected, nbytes, elapsed_s):
    """The budget that would have sufficed: the whole size at the rate observed so far (never less than what was already
    spent: more bytes than expected means the expectation was low). None when nothing moved."""
    if nbytes <= 0 or elapsed_s <= 0:
        return None
    return math.ceil(round(max(expected, nbytes) * elapsed_s / nbytes, 6))


def format_budget_note(budget_s, source, nbytes, unclamped_s):
    """Where the budget comes from, for the stage log (the text after `copy budget <n>s `)."""
    if source == "knob":
        why = "set by %s" % KNOB
    else:
        why = "derived: %d bytes at the floor of %g MB/s (MEASURED 2026-11-20 on one box), +%d%% and %d s" % (
            nbytes, FLOOR_RATE_BYTES_S / 1e6, MARGIN_PCT, FIXED_S)
    if budget_s < unclamped_s:
        why += "; clamped to the time left (wanted %ds)" % unclamped_s
    return why


def format_progress(nbytes, elapsed_s, budget_s, final=False):
    line = "SPEC_NOTE copy progress bytes=%d rate_mb_s=%.1f elapsed_s=%d budget_s=%d" % (
        nbytes, rate_mb_s(nbytes, elapsed_s), int(elapsed_s), budget_s)
    return line + (" final=1" if final else "")


def format_abort(nbytes, elapsed_s, budget_s, expected):
    need = needed_s(expected, nbytes, elapsed_s)
    return (
        "the copy into the guest did not finish inside its budget: bytes=%d expected_bytes=%d rate_mb_s=%.1f elapsed_s=%d budget_s=%d"
        " needed_s≈%s (%s at least that, plus a margin, lets this copy finish)"
        % (nbytes, expected, rate_mb_s(nbytes, elapsed_s), int(elapsed_s), budget_s, "unknown" if need is None else need, KNOB + "=")
    )


def read_status(path):
    """(bytes, elapsed_s, final) of a status file; (0, 0.0, False) when it is absent or not ours."""
    try:
        m = STATUS_RE.match(Path(path).read_text(encoding="ascii", errors="replace").strip())
    except OSError:
        m = None
    return (int(m.group(1)), float(m.group(2)), m.group(3) == "1") if m else (0, 0.0, False)


class Meter:
    """The byte counter of the copy. A ticker thread rewrites the status file about every second (so that a copy blocked in a
    write still has a fresh status) and appends a progress line every `interval` seconds."""

    def __init__(self, status, log, budget_s, expected, interval, clock=time.monotonic):
        self.status, self.log, self.budget_s, self.expected, self.interval = status, log, budget_s, expected, interval
        self.clock, self.t0, self.bytes = clock, clock(), 0
        self.stop = threading.Event()
        self.ticker = threading.Thread(target=self._tick, daemon=True)

    def elapsed(self):
        return self.clock() - self.t0

    def _write_status(self, final):
        tmp = "%s.tmp" % self.status
        try:
            Path(tmp).write_text("COPY_PROGRESS bytes=%d elapsed_s=%.1f final=%d\n" % (self.bytes, self.elapsed(), 1 if final else 0), encoding="ascii")
            os.replace(tmp, self.status)
        except OSError:
            pass  # advisory

    def _append_note(self, final):
        line = format_progress(self.bytes, self.elapsed(), self.budget_s, final) + "\n"
        try:
            fd = os.open(self.log, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
            try:
                os.write(fd, line.encode("ascii"))  # one small write: an atomic append beside the stage's own notes
            finally:
                os.close(fd)
        except OSError:
            pass  # advisory

    def _tick(self):
        due = self.interval
        while not self.stop.wait(min(1.0, self.interval / 4)):
            self._write_status(False)
            if self.elapsed() >= due:
                self._append_note(False)
                due += self.interval

    def start(self):
        self._write_status(False)
        self.ticker.start()

    def finish(self):
        self.stop.set()
        self.ticker.join()
        self._write_status(True)
        self._append_note(True)


def run_pipe(a):
    meter = Meter(a.status, a.log, a.budget, a.expected, a.interval)
    meter.start()
    rc = 0
    try:
        while True:
            chunk = os.read(0, CHUNK)
            if not chunk:
                break
            view = memoryview(chunk)
            while view:
                n = os.write(1, view)
                meter.bytes += n
                view = view[n:]
    except BrokenPipeError:
        rc = 1  # the guest's tar ended or was stopped: the stage reads its status, this is not news
    except OSError as e:
        print("copy_budget: %s" % type(e).__name__, file=sys.stderr)
        rc = 1
    finally:
        meter.finish()
    return rc


def out(text):
    sys.stdout.buffer.write((text + "\n").encode("utf-8"))
    sys.stdout.buffer.flush()


def main(argv=None):
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("--budget", default="")
    c.add_argument("--note-s", default="")
    b = sub.add_parser("budget")
    b.add_argument("--lane", required=True)
    b.add_argument("--knob", default="")
    b.add_argument("--left", type=int, required=True)
    p = sub.add_parser("pipe")
    p.add_argument("--status", required=True)
    p.add_argument("--log", required=True)
    p.add_argument("--budget", type=int, required=True)
    p.add_argument("--expected", type=int, required=True)
    p.add_argument("--note-s", default="")
    p.add_argument("--interval", type=float, default=None, help="seconds between notes, a fraction allowed (tests); else --note-s")
    t = sub.add_parser("abort-text")
    t.add_argument("--status", required=True)
    t.add_argument("--budget", type=int, required=True)
    t.add_argument("--expected", type=int, required=True)
    a = ap.parse_args(argv)
    if a.cmd == "check":
        try:
            parse_knob(a.budget)
            parse_note_s(a.note_s)
        except KnobError as e:
            out(str(e))
            return 2
        return 0
    if a.cmd == "budget":
        try:
            got = budget(a.lane, a.knob, a.left)
        except (KnobError, ValueError, KeyError, OSError) as e:
            print("copy_budget: %s" % (e if isinstance(e, KnobError) else "unknown lane or unreadable lane table"), file=sys.stderr)
            return 2
        out("%d %s %d %d %s" % (got + (format_budget_note(*got),)))
        return 0
    if a.cmd == "pipe":
        try:
            a.interval = a.interval if a.interval is not None else parse_note_s(a.note_s)
        except KnobError as e:
            print("copy_budget: %s" % e, file=sys.stderr)
            return 2
        return run_pipe(a)
    nbytes, elapsed, _ = read_status(a.status)
    out(format_abort(nbytes, elapsed, a.budget, a.expected))
    return 0


if __name__ == "__main__":
    sys.exit(main())
