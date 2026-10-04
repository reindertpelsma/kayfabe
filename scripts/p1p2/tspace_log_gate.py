#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""P1+P2 box-log gate (docs/design/V3_P1P2_TSPACE.md §8): read one kf3 boot log and say, by name,
whether the T-space properties held on that run.

  tspace_log_gate.py LOG              # a KF3_TSPACE=1 run: every check must pass
  tspace_log_gate.py LOG --windows    # the KF3_TSPACE=0 arm: the KNOWN POSITIVE - mirrors DO
                                      # carry windows; proves the gate can see them at all
  tspace_log_gate.py LOG --negctl-stale  # a KF3_NEGCTL_STALE_BIND=1 run: stale_binds MUST move

Checks on a T-space run (each one names itself in the output):
  T-TSPACE-BUILD  the `tspace` line exists, both windows end at or below the ring region, and it
                  precedes the first `BORN Translated`;
  WINDOWS=NONE    every mirror and every prewarmed spare logs `windows=none`; any `windows fb=`
                  line fails (S1-21);
  COUNTERS        the last status line's tspace[...] is built=yes with every refusal and leak at 0;
  STALE-BIND      every TSPACE-RETIRE line has stale_binds=0/N (or, with --negctl-stale, at least
                  one moves);
  NO-DEAD-BIND    no Translated channel died at a T-mode bind;
  USER-BIRTHS     at least one passthrough birth with privilege=0 (else T-WINDOW-USER did not test
                  an unprivileged guest user).
The exit status is the number of failed checks (0 = pass). An empty or unreadable log is a FAILURE
of its own, never a pass: absence of evidence is not evidence.
"""

from __future__ import annotations

import re
import sys

RING_REGION_BASE = (1 << 40) - (4 << 30)

TSPACE_RE = re.compile(r"tspace space=(0x[0-9a-f]+) fb=(0x[0-9a-f]+)\+(0x[0-9a-f]+) "
                       r"ram=(0x[0-9a-f]+)\+(0x[0-9a-f]+).*build_us=(\d+)")
MIRROR_RE = re.compile(r"mirror space=0x[0-9a-f]+: (.*)")
SPARE_RE = re.compile(r"prewarm: spare host space 0x[0-9a-f]+ ready before the guest runs (.*)")
BORN_T_RE = re.compile(r"BORN Translated")
BORN_P_RE = re.compile(r"BORN Passthrough.*privilege=(\S+)")
COUNTERS_RE = re.compile(r"tspace\[built=(\S+) twin_refused=(\d+) tspace_refused=(\d+) slots_leaked=(\d+)\]")
RETIRE_RE = re.compile(r"TSPACE-RETIRE .*stale_binds=(\d+)/(\d+)")
DEAD_BIND_RE = re.compile(r"DEAD: tspace bind:")


def gate(lines: list[str], windows: bool = False, negctl_stale: bool = False) -> list[tuple[str, bool, str]]:
    """Return `(check, passed, detail)` per check."""
    out: list[tuple[str, bool, str]] = []
    if not lines:
        return [("LOG", False, "empty log: nothing was recorded, which is not a pass")]
    mirrors = [m.group(1) for ln in lines if (m := MIRROR_RE.search(ln))]
    spares = [m.group(1) for ln in lines if (m := SPARE_RE.search(ln))]
    with_windows = [x for x in mirrors + spares if "windows fb=" in x or "fb_window" in x]
    if windows:
        # The KF3_TSPACE=0 arm: the known positive. The gate must SEE windows here.
        out.append(("WINDOWS-PRESENT", bool(with_windows),
                    f"{len(with_windows)} of {len(mirrors) + len(spares)} mirror/spare lines carry windows"))
        return out
    first_born_t = next((i for i, ln in enumerate(lines) if BORN_T_RE.search(ln)), None)
    ts = [(i, m) for i, ln in enumerate(lines) if (m := TSPACE_RE.search(ln))]
    if not ts:
        out.append(("T-TSPACE-BUILD", False, "no `tspace space=` line"))
    else:
        i, m = ts[0]
        fb_end = int(m.group(2), 16) + int(m.group(3), 16)
        ram_end = int(m.group(4), 16) + int(m.group(5), 16)
        ok = fb_end <= RING_REGION_BASE and ram_end <= RING_REGION_BASE and (first_born_t is None or i < first_born_t)
        out.append(("T-TSPACE-BUILD", ok,
                    f"fb ends {fb_end:#x}, ram ends {ram_end:#x}, line {i + 1}, first BORN Translated "
                    f"{'none' if first_born_t is None else first_born_t + 1}, build_us={m.group(6)}"))
    none_ok = bool(mirrors) and not with_windows and all("windows=none" in x for x in mirrors + spares)
    out.append(("WINDOWS=NONE", none_ok,
                f"{len(mirrors)} mirrors, {len(spares)} spares, {len(with_windows)} with a window"))
    counters = [m for ln in lines if (m := COUNTERS_RE.search(ln))]
    if counters:
        c = counters[-1]
        ok = c.group(1) == "yes" and all(int(c.group(k)) == 0 for k in (2, 3, 4))
        out.append(("COUNTERS", ok, c.group(0)))
    else:
        out.append(("COUNTERS", False, "no tspace[...] status fragment"))
    retires = [(int(m.group(1)), int(m.group(2))) for ln in lines if (m := RETIRE_RE.search(ln))]
    stale = sum(s for s, _ in retires)
    checked = sum(c for _, c in retires)
    if negctl_stale:
        out.append(("STALE-BIND-NEGCTL", stale > 0, f"stale {stale} of {checked} (the positive control must move)"))
    else:
        out.append(("STALE-BIND", stale == 0, f"stale {stale} of {checked} over {len(retires)} retires"))
    dead = sum(1 for ln in lines if DEAD_BIND_RE.search(ln))
    out.append(("NO-DEAD-BIND", dead == 0, f"{dead} channels died at a T-mode bind"))
    users = [m.group(1) for ln in lines if (m := BORN_P_RE.search(ln))]
    unpriv = sum(1 for p in users if p.split("+")[0].rstrip(",") == "0")
    out.append(("USER-BIRTHS", unpriv > 0, f"{unpriv} unprivileged of {len(users)} passthrough births"))
    return out


def main(argv: list[str]) -> int:
    if not argv or argv[0].startswith("-"):
        print(__doc__)
        return 2
    try:
        with open(argv[0], errors="replace") as f:
            lines = f.read().splitlines()
    except OSError as e:
        print(f"LOG FAIL: {e}")
        return 1
    res = gate(lines, windows="--windows" in argv, negctl_stale="--negctl-stale" in argv)
    for name, ok, detail in res:
        print(f"{name} {'PASS' if ok else 'FAIL'}: {detail}")
    return sum(1 for _, ok, _ in res if not ok)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
