#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""Mutation check for the tick WIRING in the core (nvkvm_broker.c): break it one
defect at a time in a COPY of the built tree, rebuild just that object, and
demand that test/test_tick.py (the real-socket tests) fails.  Not part of
`make check` (each mutant is a rebuild + an 18 s socket run); run it with

    make mutate-tick-core
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)

# (name, file, old, new)
MUTANTS = [
    ("[equivalent: backed up by the queue pass] a queued tick is never replaced in place", "nvkvm_broker.c",
     "    if (last != ~0u && !(last == s->tx_tail && s->tx_partial != 0) &&",
     "    if (0 && last != ~0u && !(last == s->tx_tail && s->tx_partial != 0) &&"),
    ("[equivalent: backed up by the in-place replace] the queue never drops a superseded tick", "nvkvm_broker.c",
     "        if (p->type == NVKVM_BROKER_EV_TICK && here != last_tick &&\n            nb_tick_may_replace(",
     "        if (0 && p->type == NVKVM_BROKER_EV_TICK && here != last_tick &&\n            nb_tick_may_replace("),
    ("a dead WANT tick is kept behind a newer one", "nvkvm_broker.c",
     "        if (p->type == NVKVM_BROKER_EV_TICK && here != last_want &&",
     "        if (0 && p->type == NVKVM_BROKER_EV_TICK && here != last_want &&"),
    ("neither the in-place replace nor the queue pass drops a tick (they back each other up, "
     "so each alone is an EQUIVALENT mutant -- see #1 and #2 -- and only both off is a defect)",
     "nvkvm_broker.c",
     ["    if (last != ~0u && !(last == s->tx_tail && s->tx_partial != 0) &&",
      "        if (p->type == NVKVM_BROKER_EV_TICK && here != last_tick &&\n            nb_tick_may_replace("],
     ["    if (0 && last != ~0u && !(last == s->tx_tail && s->tx_partial != 0) &&",
      "        if (0 && p->type == NVKVM_BROKER_EV_TICK && here != last_tick &&\n            nb_tick_may_replace("]),
    ("COMMIT.seq is not checked", "nvkvm_broker.c",
     "if (!nb_tick_reply(s, c->seq, true)) {", "if (0) {"),
    ("NO_CHANGE is accepted without the opt-in", "nvkvm_broker.c",
     "        if (!s->tick.on) {\n            nb_violation(s, \"NO_CHANGE without CLIENT_TICK\");",
     "        if (0) {\n            nb_violation(s, \"NO_CHANGE without CLIENT_TICK\");"),
    ("CLIENT_TICK is accepted on a broker that did not advertise it", "nvkvm_broker.c",
     "    if (!(s->sess->caps & NVKVM_BROKER_CAP_TICK)) {", "    if (0) {"),
    ("the VMM's max Hz is not range checked", "nvkvm_broker.c",
     "    if (c->height != 0 && (c->height < NVKVM_BROKER_GUEST_MIN_HZ ||\n                           c->height > NVKVM_BROKER_GUEST_MAX_HZ)) {",
     "    if (0) {"),
    ("CAPS height without CLIENT_TICK is accepted", "nvkvm_broker.c",
     "        if (c->height != 0) {\n            nb_violation(s, \"CAPS height without CLIENT_TICK\");",
     "        if (0) {\n            nb_violation(s, \"CAPS height without CLIENT_TICK\");"),
    ("no priming tick at the opt-in", "nvkvm_broker.c",
     "    nb_tick_prime(&s->tick, nb_sess_now_ns(s), &o);\n    nb_emit_tick(s, &o);\n    return true;",
     "    (void)o;\n    return true;"),
    ("the mode rate is not re-advertised after the divisor", "nvkvm_broker.c",
     "        if (mhz != s->surf_mhz) {\n            nb_sink_surface(s, s->sess->width, s->sess->height, mhz);",
     "        if (0) {\n            nb_sink_surface(s, s->sess->width, s->sess->height, mhz);"),
    ("a failed COMMIT leaves the frame 'un-presented'", "nvkvm_broker.c",
     "        if (r != 0) {\n            nb_sink_presented(s);",
     "        if (0) {\n            nb_sink_presented(s);"),
    ("tick state survives the connection", "nvkvm_broker.c",
     "    nb_tick_reset(&s->tick, false, 0);\n    s->surf_mhz = 0;",
     "    s->surf_mhz = 0;"),
    ("the NO_CHANGE record's other fields are not checked", "nvkvm_broker.c",
     "        if (c->flags || c->height || c->stride || c->offset || c->fourcc ||\n            c->modifier || c->seq) {\n            nb_violation(s, \"NO_CHANGE carried other fields\");",
     "        if (0) {\n            nb_violation(s, \"NO_CHANGE carried other fields\");"),
]


def main():
    bad = 0
    print("tick core mutation check: %d mutants" % len(MUTANTS))
    for i, (name, fname, old, new) in enumerate(MUTANTS, 1):
        with tempfile.TemporaryDirectory(prefix="muttc") as tmp:
            work = os.path.join(tmp, "broker")
            shutil.copytree(SRC, work, symlinks=True,
                            ignore=shutil.ignore_patterns("*.sock", "__pycache__"))
            path = os.path.join(work, fname)
            text = open(path).read()
            olds = old if isinstance(old, list) else [old]
            news = new if isinstance(new, list) else [new]
            if any(text.count(o) != 1 for o in olds):
                print("  ERROR  #%d %s: mutation text not found exactly once" % (i, name))
                bad += 1
                continue
            for o, n in zip(olds, news):
                text = text.replace(o, n)
            open(path, "w").write(text)
            os.utime(path, None)
            r = subprocess.run(["make", "-C", work, "nvkvm-display-broker"],
                               capture_output=True, text=True)
            if r.returncode != 0:
                print("  ERROR  #%d %s: does not build\n%s" % (i, name, r.stderr[-400:]))
                bad += 1
                continue
            r = subprocess.run([sys.executable, os.path.join(work, "test", "test_tick.py"),
                                os.path.join(work, "nvkvm-display-broker")],
                               capture_output=True, text=True, timeout=300)
            fails = [l for l in r.stdout.splitlines() if "FAIL" in l]
            ok = r.returncode != 0
            if name.startswith("[equivalent"):
                print("  %s  #%d %s -> %d failing check(s) (expected: redundant by design)"
                      % ("EQUIVALENT" if not ok else "KILLED", i, name, len(fails)))
                continue
            print("  %s  #%d %s -> %d failing check(s)%s"
                  % ("KILLED" if ok else "SURVIVED", i, name, len(fails),
                     ("  e.g. " + fails[0].strip()[:90]) if fails else ""))
            if not ok:
                bad += 1
    if bad:
        print("tick core mutation check FAILED: %d mutant(s)" % bad)
        return 1
    print("tick core mutation check: all %d mutants killed" % len(MUTANTS))
    return 0


if __name__ == "__main__":
    sys.exit(main())
