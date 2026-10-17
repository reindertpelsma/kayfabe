#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""Mutation check for the display tick (nb_tick.c): break the decisions on
purpose, one defect at a time, and demand that test-tick notices -- at least
TWO distinct failing checks per mutant (one failing check is a coincidence;
two is a net).

Each mutant is an exact textual replacement in a COPY of nb_tick.c (the real
file is never touched).  A mutant whose text is not found exactly once is
itself an error, so the table cannot rot silently when the code moves.

    python3 test/mutate_tick.py            # all mutants
    python3 test/mutate_tick.py -v         # also print the failing checks
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)

# (name, old text, new text) -- all in nb_tick.c
MUTANTS = [
    ("no decimation: every accepted feedback ticks",
     "    if (t->phase != 0) {", "    if (0) {"),
    ("the divisor rounds down (a 144 Hz host is not divided)",
     "d = (host + lim - 1u) / lim;", "d = host / lim;"),
    ("the 75 Hz slack is gone (a 75 Hz host is halved)",
     "(uint64_t)tick_cap(cap_hz) * 1000u + NB_TICK_HZ_SLACK_MHZ;",
     "(uint64_t)tick_cap(cap_hz) * 1000u;"),
    ("the client's cap is ignored",
     "uint64_t lim = (uint64_t)tick_cap(cap_hz) * 1000u + NB_TICK_HZ_SLACK_MHZ;",
     "uint64_t lim = 75u * 1000u + NB_TICK_HZ_SLACK_MHZ;"),
    ("WANT_FRAME while a request is open (never times out gate)",
     "    return now_ns - t->open_since_ns >= NVKVM_BROKER_TICK_REQ_TIMEOUT_NS;",
     "    return true;"),
    ("the request timeout edge is exclusive",
     "    return now_ns - t->open_since_ns >= NVKVM_BROKER_TICK_REQ_TIMEOUT_NS;",
     "    return now_ns - t->open_since_ns > NVKVM_BROKER_TICK_REQ_TIMEOUT_NS;"),
    ("WANT_FRAME before the answered frame is presented",
     "    if (t->unpresented &&\n        now_ns - t->unpresented_since_ns < NVKVM_BROKER_TICK_REQ_TIMEOUT_NS) {\n        return false;",
     "    if (0) {\n        return false;"),
    ("the presentation timeout edge is inclusive",
     "now_ns - t->unpresented_since_ns < NVKVM_BROKER_TICK_REQ_TIMEOUT_NS) {",
     "now_ns - t->unpresented_since_ns <= NVKVM_BROKER_TICK_REQ_TIMEOUT_NS) {"),
    ("tick_seq wraps to 0",
     "return seq == 0 ? 1u : seq;", "return seq;"),
    ("an answer for a request never issued is not a violation",
     "if (t->last_seq == 0 || nb_tick_seq_newer(req, t->last_seq)) {",
     "if (0) {"),
    ("any answer closes the open request",
     "if (t->open_req != 0 && req == t->open_req) {",
     "if (t->open_req != 0) {"),
    ("a COMMIT answer does not wait for presentation",
     "            t->unpresented = true;", "            t->unpresented = false;"),
    ("NO_CHANGE with request 0 is accepted",
     "        return NB_TICK_ANS_VIOLATION;\n    }\n    /* an id the broker never issued",
     "        return NB_TICK_ANS_UNSOLICITED;\n    }\n    /* an id the broker never issued"),
    ("flood: no duplicate / too-fast rule",
     "    if (t->have_accept &&\n", "    if (0 && t->have_accept &&\n"),
    ("flood: the half-period edge is inclusive",
     "now_ns - t->last_accept_ns < host_period_ns / 2)) {",
     "now_ns - t->last_accept_ns <= host_period_ns / 2)) {"),
    ("flood: compositor timestamps are not judged",
     "    if (present_ns != 0 && !(flags & NVKVM_BROKER_TICK_F_CLOCK_ONLY)) {",
     "    if (0) {"),
    ("TX queue: a WANT tick may be replaced",
     "    return (queued_refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME) == 0;",
     "    return true;"),
    ("TX queue: a tick is never replaceable",
     "    return (queued_refresh_word & NVKVM_BROKER_TICK_F_WANT_FRAME) == 0;",
     "    return false;"),
    ("present_ns may decrease",
     "    if (present_ns < t->last_present_ns) {", "    if (0) {"),
    ("caller flags leak into the word",
     "    word |= flags & NVKVM_BROKER_TICK_F_CLOCK_ONLY;", "    word |= flags;"),
    ("an out-of-range period is sent as it is",
     "    if (period_ns >= NVKVM_BROKER_TICK_REFRESH_MIN_NS &&\n        period_ns <= NVKVM_BROKER_TICK_REFRESH_MASK) {",
     "    if (1) {\n        period_ns &= NVKVM_BROKER_TICK_REFRESH_MASK;"),
    ("a changed rate does not restart the count",
     "        t->phase = div - 1u;", "        t->phase = 0;"),
    ("the priming tick does not want a frame",
     "NVKVM_BROKER_TICK_F_CLOCK_ONLY, true, o);", "NVKVM_BROKER_TICK_F_CLOCK_ONLY, false, o);"),
    ("a WANT tick does not open its request",
     "        t->open_req = t->last_seq;", "        t->open_req = t->open_req;"),
    ("a late answer closes nothing but a late one counts as open",
     "    t->n_late++;\n    return NB_TICK_ANS_LATE;", "    return NB_TICK_ANS_OPEN;"),
    ("presented does not clear",
     "    t->unpresented = false;\n}\n\nbool nb_tick_may_replace",
     "}\n\nbool nb_tick_may_replace"),
    ("a seq compare that ignores the wrap",
     "return (int32_t)(a - b) > 0;", "return a > b;"),
]


def build_and_run(workdir):
    cc = os.environ.get("CC", "cc")
    exe = os.path.join(workdir, "t")
    cmd = [cc, "-O1", "-g", "-std=gnu11", "-w", "-D_GNU_SOURCE", "-I", workdir,
           "-I", os.path.join(workdir, "common"), "-o", exe,
           os.path.join(workdir, "test_tick.c"), os.path.join(workdir, "nb_tick.c")]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return None, r.stderr
    try:
        r = subprocess.run([exe], capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return ["TIMEOUT"], ""
    fails = [l for l in r.stderr.splitlines() if l.startswith("FAIL ")]
    if r.returncode != 0 and not fails:
        fails = ["CRASH rc=%d" % r.returncode]
    return fails, ""


def main():
    verbose = "-v" in sys.argv
    bad = 0
    print("tick mutation check: %d mutants" % len(MUTANTS))
    for i, (name, old, new) in enumerate(MUTANTS, 1):
        with tempfile.TemporaryDirectory(prefix="muttick") as tmp:
            for f in ("nvkvm_broker.h", "nb_tick.c"):
                shutil.copy(os.path.join(SRC, f), tmp)
            shutil.copytree(os.path.join(SRC, "common"), os.path.join(tmp, "common"))
            shutil.copy(os.path.join(HERE, "test_tick.c"), tmp)
            path = os.path.join(tmp, "nb_tick.c")
            text = open(path).read()
            if text.count(old) != 1:
                print("  ERROR  #%d %s: mutation text found %d times"
                      % (i, name, text.count(old)))
                bad += 1
                continue
            open(path, "w").write(text.replace(old, new))
            fails, err = build_and_run(tmp)
            if fails is None:
                print("  ERROR  #%d %s: does not compile\n%s" % (i, name, err[:400]))
                bad += 1
                continue
            kinds = {re.sub(r"[0-9]+", "N", re.sub(r"^FAIL [^ ]+ ", "", l)) for l in fails}
            ok = len(kinds) >= 2
            print("  %s  #%d %s -> %d failing check(s), %d distinct"
                  % ("KILLED" if ok else "SURVIVED/WEAK", i, name, len(fails), len(kinds)))
            if verbose:
                for k in sorted(kinds)[:4]:
                    print("          " + k[:150])
            if not ok:
                bad += 1
    if bad:
        print("tick mutation check FAILED: %d mutant(s) not killed twice" % bad)
        return 1
    print("tick mutation check: all %d mutants killed by >= 2 distinct checks" % len(MUTANTS))
    return 0


if __name__ == "__main__":
    sys.exit(main())
