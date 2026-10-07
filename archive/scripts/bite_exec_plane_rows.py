#!/usr/bin/env python3
"""Bite harness for #155's execution-plane triage rows (`0xa06f0104`, `0xc36f0108`).

★★★ The thing under test here is a **table**, not a function, so the question a bite has to
answer is different: *does anything actually READ these rows?* A row nothing reads is
documentation that compiles, and this project has measured that failure mode (`t134a`: "we
do not serve `0x20800a1c`" was the *absence* of a variant and nothing could say so).

★★ Every mutation is chosen to **COMPILE**. A mutation that breaks the build is not a bite —
the test never ran. `COMPILE-FAIL` is reported as a distinct, failing outcome.

★★ Files are restored with an explicit `os.utime` bump: `shutil.copy2`/`cp -a` preserve the
OLD mtime, cargo then serves a stale rlib, and every later mutation reads as a non-biter —
measured twice on 2026-07-28.

⊘ These mutations do **not** probe whether the rows' arguments are *true*. That is what the
boot did (`docs/design/boot_measured_2026_08_01.md` §44); this only shows the rows are
load-bearing rather than inert prose.

Usage:  python3 scripts/bite_exec_plane_rows.py [--cargo-target DIR]
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

SWEEP = "crates/kayfabe-device/src/sweep.rs"
TRIAGE = "-p kayfabe-device --test sweep_triage"

# (name, file, old, new, cargo-args, expected-test)
MUTATIONS = [
    (
        "0xa06f0104's disposition downgraded to AmputationIntended — 'the bind is an "
        "engine this chip lacks', which would make refusing it correct",
        SWEEP,
        "        cmd: 0xa06f_0104,\n"
        '        engine: "KernelChannel (global CeUtils)",\n'
        "        disposition: SweepDisposition::RefusalHalts,",
        "        cmd: 0xa06f_0104,\n"
        '        engine: "KernelChannel (global CeUtils)",\n'
        "        disposition: SweepDisposition::AmputationIntended,",
        TRIAGE,
        "a_halting_refusal_may_be_served_or_not_and_the_table_says_which",
    ),
    (
        "0xc36f0108's disposition raised to AmputationUnsurvivable — the must-serve gate "
        "should then demand a control this port deliberately does not serve",
        SWEEP,
        "        cmd: 0xc36f_0108,\n"
        '        engine: "KernelChannel (CE scrubber)",\n'
        "        disposition: SweepDisposition::RefusalHalts,",
        "        cmd: 0xc36f_0108,\n"
        '        engine: "KernelChannel (CE scrubber)",\n'
        "        disposition: SweepDisposition::AmputationUnsurvivable,",
        TRIAGE,
        "a_refusal_this_port_may_not_make_is_never_left_unserved",
    ),
    (
        "0xa06f0104's id silently changed to the BIND control's neighbour — the universe "
        "stays the same SIZE, so only a membership pin can see it",
        SWEEP,
        "        cmd: 0xa06f_0104,\n"
        '        engine: "KernelChannel (global CeUtils)",',
        "        cmd: 0xa06f_0105,\n"
        '        engine: "KernelChannel (global CeUtils)",',
        TRIAGE,
        "the_triage_universe_is_pinned_so_shortening_it_is_a_red_test",
    ),
    (
        "EVERY citation stripped from 0xc36f0108's argument — a row that decides without "
        "naming a source. \u26a0 A first attempt at this bite removed only the FIRST of the "
        "row's two `ogkm-580:` tags and SURVIVED, correctly: the gate is satisfied by one "
        "citation anywhere in the string. That is the `a gate keyed on a WORD is satisfied "
        "by writing the word` shape, recorded here rather than silently worked around",
        SWEEP,
        'why: "NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN (ogkm-580: ctrlc36f.h:79), one \\\n'
        "              [OUT] NvU32 workSubmitToken (:83-85). Reached from mem_utils.c:2024 via \\\n"
        "              kfifoRmctrlGetWorkSubmitToken_GV100, which returns rmStatus VERBATIM \\\n"
        "              (ogkm-580: kernel_fifo_gv100.c:86-93) \u2014 so unlike its two neighbours in \\",
        'why: "NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, one \\\n'
        "              [OUT] NvU32 workSubmitToken. Reached from the scrubber setup via \\\n"
        "              kfifoRmctrlGetWorkSubmitToken_GV100, which returns rmStatus VERBATIM \\\n"
        "              \u2014 so unlike its two neighbours in \\",
        TRIAGE,
        "every_triaged_control_carries_an_argument_and_cites_something",
    ),
]


def run(args, target):
    env = dict(os.environ)
    if target:
        env["CARGO_TARGET_DIR"] = target
    return subprocess.run(
        ["cargo", "test"] + args.split() + ["--", "--nocapture"],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cargo-target", default=os.environ.get("CARGO_TARGET_DIR"))
    a = ap.parse_args()

    base = run(TRIAGE, a.cargo_target)
    if base.returncode != 0:
        print("★ BASELINE IS RED — nothing below is evidence about anything.")
        print(base.stdout[-4000:])
        return 2

    bit = 0
    for name, rel, old, new, cargo_args, expected in MUTATIONS:
        path = os.path.join(ROOT, rel)
        original = open(path, encoding="utf-8").read()
        if old not in original:
            print(f"UNANCHORED  {name}\n    (the text this bite is written against is gone)")
            continue
        try:
            open(path, "w", encoding="utf-8").write(original.replace(old, new, 1))
            os.utime(path, (time.time(), time.time()))
            r = run(cargo_args, a.cargo_target)
            out = r.stdout + r.stderr
            if "error[E" in out or "error: could not compile" in out:
                print(f"COMPILE-FAIL  {name}")
            elif r.returncode == 0:
                print(f"SURVIVED    {name}")
            elif expected in out:
                print(f"BIT         {name}\n    killed: {expected}")
                bit += 1
            else:
                print(f"BIT-WRONG   {name}\n    red, but NOT {expected}")
        finally:
            open(path, "w", encoding="utf-8").write(original)
            os.utime(path, (time.time(), time.time()))

    print(f"\n{bit}/{len(MUTATIONS)} mutations bitten by the test they were aimed at")
    return 0 if bit == len(MUTATIONS) else 1


if __name__ == "__main__":
    sys.exit(main())
