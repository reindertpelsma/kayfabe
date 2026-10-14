#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""P1+P2 box-log gate (docs/design/V3_P1P2_TSPACE.md §8): read one kf3 boot log and say, by name,
whether the properties a box step owes held on that run.

  tspace_log_gate.py LOG [--user] [--p0-pending]   # a KF3_TSPACE=1 run (steps 3, 4, 5)
  tspace_log_gate.py LOG --default                  # step 1: the default path, KF3_TSPACE unset
  tspace_log_gate.py LOG --census                   # step 2: the default path with KF3_TSHADOW=1
  tspace_log_gate.py LOG --windows                  # the KF3_TSPACE=0 arm: the KNOWN POSITIVE -
                                                    # mirrors DO carry windows (the gate can see them)
  tspace_log_gate.py LOG --negctl NAME              # a positive-control run: NAME's counter MUST move
      NAME: stale (KF3_NEGCTL_STALE_BIND), shadow (KF3_NEGCTL_SHADOW), heap (KF3_NEGCTL_HEAP),
            carve (KF3_NEGCTL_CARVE), twin (KF3_NEGCTL_TWIN), oversize (KF3_NEGCTL_TSPACE_OVERSIZE),
            window (KF3_NEGCTL_TWIN_WINDOW: WINDOWS=NONE must FAIL on it)

⊘ 2026-10-10 (OWNER_RULINGS.md §AB): KF3_TSPACE is deleted and the T-space is hardwired. Every
log from a build after that date is a T-space run (the plain invocation). --default, --census and
--windows describe arms that no longer exist (KF3_TSPACE unset / =0); they are kept only to read
logs from older builds, and --default FAILS on a new log by design. --negctl shadow has nothing to
control (the shadow ran only beside the deleted legacy rewriter; KF3_TSHADOW now turns on the
census only). The known positive for WINDOWS=NONE is --negctl window (KF3_NEGCTL_TWIN_WINDOW, kept).

⚠ Every T-space and census run must end with the guest driver UNLOADED (rmmod nvidia_uvm
nvidia_drm nvidia_modeset nvidia, or a clean shutdown) before the log is taken: TSPACE-RETIRE,
TCENSUS and TSHADOW lines are printed when a Translated channel is FREED, and CeUtils' channels
live until the adapter is torn down. Without that step the retire/census checks fail by design.

Checks on a T-space run (each names itself in the output):
  T-TSPACE-BUILD  the `tspace` line exists, both windows end at or below the ring region, and it
                  precedes the first `BORN Translated`;
  BORN-TRANSLATED at least one Translated channel was born (else nothing above was exercised);
  WINDOWS=NONE    every mirror (created or recycled) and every prewarmed spare logs `windows=none`;
                  any line naming a window fails (S1-21);
  COUNTERS        the last status line: tspace[built=yes twin_refused=0 tspace_refused=0
                  slots_leaked=0], carve_gpu=0 (no twin a guest non-kernel channel runs in mapped
                  the carve-out), inca[... heap_out=0 ...];
  STALE-BIND      at least one TSPACE-RETIRE re-checked a resolution (checked > 0) and none was
                  stale;
  NO-DEAD         no Translated channel died (any `DEAD:` on a token born Translated);
  P0-EVIDENCE     every born host channel carries the host reply's USER evidence (P0's
                  `kf-host: channel birth ... PRIVILEGED_CHANNEL=0` line) and none is privileged;
                  without P0 in the build the run FAILS here unless --p0-pending, which passes it
                  SCOPED: "window and carve-out reach only; the channel-privilege half waits on P0";
  USER-BIRTHS     (--user) at least one passthrough birth with privilege=0.
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
BORN_T_RE = re.compile(r"BORN Translated: token (0x[0-9a-f]+)")
BORN_P_RE = re.compile(r"BORN Passthrough: token (0x[0-9a-f]+).*privilege=(\S+)")
BORN_ANY_RE = re.compile(r"BORN (Translated|Passthrough)")
COUNTERS_RE = re.compile(r"tspace\[built=(\S+) twin_refused=(\d+) tspace_refused=(\d+) slots_leaked=(\d+)"
                         r"(?: twin_freeing=(\d+))?\]")
STATUS_RE = re.compile(r"mem\[inval=")
CARVE_RE = re.compile(r"carve_gpu=(\d+) carve_kernel=(\d+) carve_cpu=(\d+)")
INCA_RE = re.compile(r"inca\[strict=(\S+) counted=(\d+) heap_out=(\d+) rows_inexact=(\d+)\]")
RETIRE_RE = re.compile(r"TSPACE-RETIRE .*stale_binds=(\d+)/(\d+)")
DEAD_RE = re.compile(r"chan token (0x[0-9a-f]+) \(.*\) DEAD:")
P0_USER_RE = re.compile(r"kf-host: channel birth h=\S+ .*PRIVILEGED_CHANNEL=0 privilege=USER")
P0_PRIV_RE = re.compile(r"PRIVILEGED_CHANNEL=1")
TCENSUS_RE = re.compile(r"TCENSUS tok=\S+ host=\S+ key=VasKey\((0x[0-9a-f_]+)\)")
TSHADOW_RE = re.compile(r"TSHADOW .*max_pieces=(\d+) would_refuse=\[([^\]]*)\] resolve_miss=(\d+) "
                        r"unknown_field=(\d+) unclassified=(\d+)")
NOT_BUILT_RE = re.compile(r"tspace: not built \((.*)\)")

NEGCTLS = ("stale", "shadow", "heap", "carve", "twin", "oversize", "window")


def window_lines(lines: list[str]) -> tuple[list[str], list[str], list[str]]:
    """(mirror lines, spare lines, those naming a window)."""
    mirrors = [m.group(1) for ln in lines if (m := MIRROR_RE.search(ln))]
    spares = [m.group(1) for ln in lines if (m := SPARE_RE.search(ln))]
    with_windows = [x for x in mirrors + spares if "windows fb=" in x or "fb_window" in x]
    return mirrors, spares, with_windows


def last(regex: re.Pattern, lines: list[str]):
    found = [m for ln in lines if (m := regex.search(ln))]
    return found[-1] if found else None


def retire_counts(lines: list[str]) -> tuple[int, int, int]:
    """(retires, stale, checked) over every TSPACE-RETIRE line."""
    r = [(int(m.group(1)), int(m.group(2))) for ln in lines if (m := RETIRE_RE.search(ln))]
    return len(r), sum(s for s, _ in r), sum(c for _, c in r)


def negctl_check(lines: list[str], name: str) -> tuple[str, bool, str]:
    """One positive control: its counter must have MOVED on this run."""
    if name == "stale":
        n, stale, checked = retire_counts(lines)
        return ("STALE-BIND-NEGCTL", stale > 0, f"stale {stale} of {checked} over {n} retires (must move)")
    if name == "shadow":
        rows = [m for ln in lines if (m := TSHADOW_RE.search(ln))]
        moved = [r for r in rows if int(r.group(3)) > 0 and int(r.group(4)) > 0 and int(r.group(5)) > 0]
        return ("SHADOW-NEGCTL", bool(moved), f"{len(moved)} of {len(rows)} TSHADOW lines moved resolve_miss, "
                "unknown_field and unclassified")
    if name == "heap":
        inca = last(INCA_RE, lines)
        born = sum(1 for ln in lines if BORN_ANY_RE.search(ln))
        ok = inca is not None and int(inca.group(3)) > 0 and born > 0
        return ("HEAP-NEGCTL", ok, f"{inca.group(0) if inca else 'no inca[...]'}; {born} births (counted, never refused)")
    if name == "carve":
        c = last(CARVE_RE, lines)
        ok = c is not None and sum(int(c.group(k)) for k in (1, 2, 3)) > 0
        return ("CARVE-NEGCTL", ok, c.group(0) if c else "no carve_gpu= on any status line")
    if name == "twin":
        c = last(COUNTERS_RE, lines)
        return ("TWIN-NEGCTL", c is not None and int(c.group(2)) > 0, c.group(0) if c else "no tspace[...]")
    if name == "oversize":
        nb = [m.group(1) for ln in lines if (m := NOT_BUILT_RE.search(ln))]
        c = last(COUNTERS_RE, lines)
        ok = any("reaches the ring region" in x for x in nb) and c is not None and int(c.group(3)) > 0
        return ("OVERSIZE-NEGCTL", ok, f"not built: {nb[:1]}; {c.group(0) if c else 'no tspace[...]'}")
    if name == "window":
        mirrors, spares, with_windows = window_lines(lines)
        return ("WINDOW-NEGCTL", bool(with_windows),
                f"{len(with_windows)} of {len(mirrors) + len(spares)} mirror/spare lines name a window "
                "(WINDOWS=NONE must fail on this run)")
    return ("NEGCTL", False, f"unknown positive control {name!r}; one of {', '.join(NEGCTLS)}")


def default_checks(lines: list[str], census: bool) -> list[tuple[str, bool, str]]:
    """Steps 1 and 2: the default path, count-only — every inc-A counter 0, no T-mode line."""
    out = []
    inca = last(INCA_RE, lines)
    if inca is None:
        out.append(("INCA-COUNTERS", False, "no inca[...] status fragment"))
    else:
        ok = inca.group(1) == "no" and all(int(inca.group(k)) == 0 for k in (2, 3, 4))
        out.append(("INCA-COUNTERS", ok, inca.group(0)))
    tmode = [ln for ln in lines if TSPACE_RE.search(ln) or "TSPACE-RETIRE" in ln]
    out.append(("DEFAULT-PATH", not tmode, f"{len(tmode)} T-space lines (KF3_TSPACE must be unset)"))
    carve = last(CARVE_RE, lines)
    out.append(("CARVE-RECORDED", carve is not None,
                (carve.group(0) + " (the A2 input; recorded, not gated)") if carve else "no carve_gpu= on any status line"))
    if census:
        keys = [m.group(1) for ln in lines if (m := TCENSUS_RE.search(ln))]
        rm_internal = [k for k in keys if k.startswith("0xc1e")]
        out.append(("TCENSUS", bool(rm_internal),
                    f"{len(keys)} TCENSUS lines, {len(rm_internal)} from an RM-internal channel (CeUtils)"))
        rows = [m for ln in lines if (m := TSHADOW_RE.search(ln))]
        bad = [r.group(0) for r in rows
               if int(r.group(1)) > 3 or r.group(2).strip() or any(int(r.group(k)) for k in (3, 4, 5))]
        out.append(("TSHADOW", bool(rows) and not bad,
                    f"{len(rows)} TSHADOW lines; {len(bad)} with a would-refuse, a miss, an unknown field, "
                    f"an unclassified method or more than 3 pieces{': ' + bad[0] if bad else ''}"))
    return out


def gate(lines: list[str], windows: bool = False, negctl: str | None = None, user: bool = False,
         p0_pending: bool = False, default: bool = False, census: bool = False) -> list[tuple[str, bool, str]]:
    """Return `(check, passed, detail)` per check."""
    if not lines:
        return [("LOG", False, "empty log: nothing was recorded, which is not a pass")]
    if negctl is not None:
        return [negctl_check(lines, negctl)]
    if windows:
        # The KF3_TSPACE=0 arm: the known positive. The gate must SEE windows here.
        mirrors, spares, with_windows = window_lines(lines)
        return [("WINDOWS-PRESENT", bool(with_windows),
                 f"{len(with_windows)} of {len(mirrors) + len(spares)} mirror/spare lines carry windows")]
    if default or census:
        return default_checks(lines, census)
    out: list[tuple[str, bool, str]] = []
    born_t = [(i, m.group(1)) for i, ln in enumerate(lines) if (m := BORN_T_RE.search(ln))]
    first_born_t = born_t[0][0] if born_t else None
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
    out.append(("BORN-TRANSLATED", bool(born_t), f"{len(born_t)} Translated births"))
    mirrors, spares, with_windows = window_lines(lines)
    none_ok = bool(mirrors) and not with_windows and all("windows=none" in x for x in mirrors + spares)
    out.append(("WINDOWS=NONE", none_ok,
                f"{len(mirrors)} mirrors, {len(spares)} spares, {len(with_windows)} with a window"))
    c = last(COUNTERS_RE, lines)
    carve = last(CARVE_RE, lines)
    inca = last(INCA_RE, lines)
    if c is None or carve is None or inca is None:
        out.append(("COUNTERS", False, "missing tspace[...], carve_gpu= or inca[...] on the status line"))
    else:
        ok = (c.group(1) == "yes" and all(int(c.group(k) or 0) == 0 for k in (2, 3, 4, 5))
              and int(carve.group(1)) == 0 and int(inca.group(3)) == 0)
        out.append(("COUNTERS", ok, f"{c.group(0)} {carve.group(0)} {inca.group(0)}"))
    n, stale, checked = retire_counts(lines)
    out.append(("STALE-BIND", n > 0 and checked > 0 and stale == 0,
                f"stale {stale} of {checked} over {n} retires (unload the guest driver first: "
                "retire lines print at free)"))
    t_tokens = {tok for _, tok in born_t}
    dead = [m.group(1) for ln in lines if (m := DEAD_RE.search(ln)) and m.group(1) in t_tokens]
    out.append(("NO-DEAD", not dead, f"{len(dead)} Translated channels died: {dead[:4]}"))
    births = sum(1 for ln in lines if BORN_ANY_RE.search(ln))
    user_ev = sum(1 for ln in lines if P0_USER_RE.search(ln))
    priv = sum(1 for ln in lines if P0_PRIV_RE.search(ln))
    if user_ev == 0 and priv == 0 and p0_pending:
        out.append(("P0-EVIDENCE(SCOPED)", True,
                    "no host birth evidence in this build — a pass is scoped to window and carve-out "
                    "reach only; the channel-privilege half waits on P0 (v3-sec-nonpriv)"))
    else:
        out.append(("P0-EVIDENCE", births > 0 and user_ev >= births and priv == 0,
                    f"{user_ev} USER host births for {births} channel births, {priv} privileged "
                    "(--p0-pending scopes a build without P0)"))
    if user:
        users = [m.group(2) for ln in lines if (m := BORN_P_RE.search(ln))]
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
    negctl = None
    if "--negctl-stale" in argv:
        negctl = "stale"
    if "--negctl" in argv:
        k = argv.index("--negctl")
        negctl = argv[k + 1] if k + 1 < len(argv) else "?"
    res = gate(lines, windows="--windows" in argv, negctl=negctl, user="--user" in argv,
               p0_pending="--p0-pending" in argv, default="--default" in argv, census="--census" in argv)
    for name, ok, detail in res:
        print(f"{name} {'PASS' if ok else 'FAIL'}: {detail}")
    failed = sum(1 for _, ok, _ in res if not ok)
    scoped = any(name.endswith("(SCOPED)") for name, _, _ in res)
    print(f"VERDICT: {'FAIL' if failed else ('PASS, SCOPED (see P0-EVIDENCE)' if scoped else 'PASS')}")
    return failed


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
