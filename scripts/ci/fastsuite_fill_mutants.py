#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of the fast suite's fault-arm device property (scripts/fastguest/run_fast_guest.sh, FAULT_ARMS).

Each mutant is one exact-once text replacement in a private copy of the fast-guest scripts; the tests of
test_fastsuite_fill.py run against the copy (`KF_FASTSUITE_ROOT`) and every mutant must fail at least two of them
by name. Stdlib only, GPU-free, no QEMU, no guest.

    python3 scripts/ci/fastsuite_fill_mutants.py [substring-of-mutant-name]
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LANE = "scripts/fastguest/run_fast_guest.sh"
SUITE = "scripts/fastguest/fast_suite.sh"

# (name, file, old text (exactly once), new text)
MUTANTS = [
    (
        "arm-drops-the-property",
        LANE,
        '[ "$_a" = "$_f" ] && FILL_ARG=",unmapped-reads-zero=off"',
        '[ "$_a" = "$_f" ] && FILL_ARG=""',
    ),
    ("only-the-first-fault-arm", LANE, "FAULT_ARMS=(--defer-liveness --missing-page-fault)", "FAULT_ARMS=(--defer-liveness)"),
    ("only-the-second-fault-arm", LANE, "FAULT_ARMS=(--defer-liveness --missing-page-fault)", "FAULT_ARMS=(--missing-page-fault)"),
    ("every-arm-gets-the-property", LANE, '[ "$_a" = "$_f" ] && FILL_ARG', '[ -n "$_a" ] && FILL_ARG'),
    ("only-the-first-arm-token-is-read", LANE, "for _a in ${ARMS_TOK//,/ }; do", "for _a in ${ARMS_TOK%%,*}; do"),
    (
        "multi-gpu-devices-lose-it",
        LANE,
        "id=kf$_i${_profile}${FILL_ARG}${KF3_DEV_EXTRA:+,$KF3_DEV_EXTRA}",
        "id=kf$_i${_profile}${KF3_DEV_EXTRA:+,$KF3_DEV_EXTRA}",
    ),
    (
        "single-device-loses-it",
        LANE,
        "id=kf0${_profile}${FILL_ARG}${KF3_DEV_EXTRA:+,$KF3_DEV_EXTRA}",
        "id=kf0${_profile}${KF3_DEV_EXTRA:+,$KF3_DEV_EXTRA}",
    ),
    (
        "explicit-on-is-not-refused",
        LANE,
        "            exit 2 ;;\n    esac\nfi\n",
        "            ;;\n    esac\nfi\n",
    ),
    (
        "any-explicit-value-counts-as-off",
        LANE,
        "*,unmapped-reads-zero=off,*) FILL_ARG=\"\" ;;",
        "*,unmapped-reads-zero=*) FILL_ARG=\"\" ;;",
    ),
]


def killers(root):
    env = dict(os.environ, KF_FASTSUITE_ROOT=str(root))
    p = subprocess.run(
        [sys.executable, "-B", str(HERE / "test_fastsuite_fill.py"), "-v"],
        env=env, capture_output=True, text=True, timeout=600,
    )
    out = p.stdout + p.stderr
    failed = {m.group(1) for m in re.finditer(r"(?m)^(?:FAIL|ERROR): (\S+) ", out)}
    if p.returncode != 0 and not failed:
        raise SystemExit("the test run failed without a failing test:\n" + out[-800:])
    return sorted(failed), p.returncode


def main():
    want = sys.argv[1:]
    weak = 0
    with tempfile.TemporaryDirectory() as tmp:
        for rel in (LANE, SUITE, "scripts/bench/guest_profile.sh", "scripts/fastguest/stranded_tokens.awk"):
            dest = Path(tmp) / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / rel, dest)
        base, rc = killers(tmp)
        if rc != 0:
            raise SystemExit("baseline is not green: %s" % base)
        print("baseline: green")
        cases = [m for m in MUTANTS if not want or any(w in m[0] for w in want)]
        for name, rel, old, new in cases:
            path = Path(tmp) / rel
            original = (ROOT / rel).read_text(encoding="utf-8")
            if original.count(old) != 1:
                raise SystemExit("mutant %s: the old text occurs %d times in %s" % (name, original.count(old), rel))
            path.write_text(original.replace(old, new), encoding="utf-8")
            try:
                failed, _ = killers(tmp)
            finally:
                path.write_text(original, encoding="utf-8")
            flag = "" if len(failed) >= 2 else "  <-- WEAK"
            print("%-36s killers=%d%s %s" % (name, len(failed), flag, failed))
            weak += len(failed) < 2
        print("MUTANTS total=%d weak=%d" % (len(cases), weak))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
