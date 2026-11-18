#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf-rm guest-log budget mutants (boundary fuzzing 2026-11-18, finding BF-2 / S1-86): every
guest-repeatable log site of the served chain is `kf_arch::rate_eprintln!`. Each mutant puts one
site back to a plain `eprintln!` and must be killed by BOTH named tests of
`crates/kf-rm/tests/guest_log_budget.rs` (the line-count bound and the running count).

The script refuses a red baseline, a missing or repeated anchor, a mutant that does not compile, and
a mutant killed by fewer than the two named killers. The mutation is applied in place and restored in
`finally` (the bytes are checked after the restore).

Run: `python3 -B crates/kf-rm/mutants/logbudget_mutants.py [name-substring]`.
"""

import os
from pathlib import Path
import re
import subprocess
import sys

CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]
KILLERS = {
    "every_guest_repeatable_refusal_line_is_rate_limited",
    "every_guest_repeatable_refusal_line_carries_its_running_count",
}

# (name, file under crates/kf-rm/src, anchor, replacement)
R = "kf_arch::rate_eprintln!(\n"
MUTANTS = [
    ("gss-census", "inittables.rs", R + '                "W343GSSCENSUS', 'eprintln!(\n                "W343GSSCENSUS'),
    ("init-table-size", "inittables.rs", R + '                "W349REFUSE cmd={:#010x} why=size ', 'eprintln!(\n                "W349REFUSE cmd={:#010x} why=size '),
    ("init-table-finn", "inittables.rs", R + '                "W349REFUSE cmd={:#010x} why=finn', 'eprintln!(\n                "W349REFUSE cmd={:#010x} why=finn'),
    ("static-info-size", "staticinfo.rs", R + "                \"kf-rm: GET_GSP_STATIC_INFO refused: the guest's GspStaticConfigInfo", "eprintln!(\n                \"kf-rm: GET_GSP_STATIC_INFO refused: the guest's GspStaticConfigInfo"),
    ("display-finn", "display.rs", R + '                "kf-rm: display: control {:#010x} arrived FINN', 'eprintln!(\n                "kf-rm: display: control {:#010x} arrived FINN'),
    ("bar-pde", "barpde.rs", R + '                    "kf-rm: UPDATE_BAR_PDE refused', 'eprintln!(\n                    "kf-rm: UPDATE_BAR_PDE refused'),
    ("video-caps", "inittables.rs", R + '                            "kf3: video caps', 'eprintln!(\n                            "kf3: video caps'),
    ("fifo-latency", "inittables.rs", R + '                                "kf3: FIFO_GET_LATENCY_BUFFER_SIZE REFUSED', 'eprintln!(\n                                "kf3: FIFO_GET_LATENCY_BUFFER_SIZE REFUSED'),
    ("fn1-undecodable", "guestsysinfo.rs", R + '                        "kf-rm: fn 1 SET_GUEST_SYSTEM_INFO: undecodable', 'eprintln!(\n                        "kf-rm: fn 1 SET_GUEST_SYSTEM_INFO: undecodable'),
]

FAILED = re.compile(r"^test (\S+) \.\.\. FAILED$", re.M)
PANIC_FAILED = re.compile(r"^---- (\S+) stdout ----$", re.M)


def run_tests():
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))
    p = subprocess.run(
        ["cargo", "test", "-q", "-p", "kf-rm", "--test", "guest_log_budget", "--no-fail-fast", "--", "--test-threads=1"],
        cwd=ROOT, env=env, capture_output=True, text=True,
    )
    out = p.stdout + p.stderr
    compiled = "error[E" not in out and "could not compile" not in out
    return p.returncode, compiled, set(FAILED.findall(out)) | set(PANIC_FAILED.findall(out)), out


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    rc, compiled, failed, out = run_tests()
    if rc != 0 or not compiled or failed:
        print(out[-4000:])
        print("BASELINE IS RED: refuse to judge mutants")
        return 2
    bad = 0
    for name, rel, anchor, repl in MUTANTS:
        if only and only not in name:
            continue
        path = CRATE / "src" / rel
        orig = path.read_bytes()
        text = orig.decode()
        n = text.count(anchor)
        if n != 1:
            print(f"ANCHOR {name}: found {n} times in {rel}")
            bad += 1
            continue
        try:
            path.write_text(text.replace(anchor, repl))
            rc, compiled, failed, out = run_tests()
        finally:
            path.write_bytes(orig)
        assert path.read_bytes() == orig, f"{rel} was not restored"
        if not compiled:
            print(f"NOCOMPILE {name}: the mutant does not build (proves nothing)")
            print(out[-2000:])
            bad += 1
        elif not KILLERS <= failed:
            print(f"SURVIVED {name}: killers={sorted(failed)}")
            bad += 1
        else:
            print(f"KILLED {name}: {', '.join(sorted(failed & KILLERS))}")
    print("mutants: " + ("FAIL" if bad else "OK"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
