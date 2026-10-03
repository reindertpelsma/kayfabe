#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""boot_gates.py <results-dir> | --files <guest.res>... — the app lane's BOOT-GATE verdict: THE ONE
RULE for reading the `APPS_BOOT_GATE` lines apps_matrix.sh appends (from boot_gate.sh) to guest.res.

★ 2026-10-03 (review of 9390f51c). `guest.res` is APPENDED to (`tee -a`), and boot tags are
`ap_<run>_b<n>` — so a reused run name leaves an earlier attempt's gate line for the same boot above
this attempt's. apps_matrix.sh counted EVERY gate line while summarize.py kept the LAST per boot, so
one results directory could exit 3 from apps_matrix.sh and print `lane=PASS` from summarize.py (or
the reverse). Now apps_matrix.sh (`guest`, `lane`), summarize.py and triage.py all call THIS module:
  - per boot, the LAST gate line wins (the newest attempt), across the files in the order given;
  - a boot that RAN apps (an APPRES row with `boot=` that is not BOOT_FAIL) but has no gate line is
    UNMEASURED — its gate never ran, which is not a pass;
  - lane=FAIL when any boot is FAIL or UNMEASURED, or no boot is counted at all; lane=UNMEASURED only
    for results that predate the gate entirely (no gate line and no row carrying `rc_none=`);
    lane=PASS otherwise.
Prints `BOOT_GATE boots=<n> pass=<p> fail=<f> unmeasured=<u> lane=PASS|FAIL|UNMEASURED` and one
`  ⊘ <boot> gate=…` line per boot that is not PASS. Exit 0 iff lane=PASS, else 3."""
import collections
import os
import re
import sys

KV_ROW = re.compile(r"(\w+)=((?:(?! \w+=).)*)")


def _lines(paths):
    for path in paths:
        if os.path.exists(path):
            yield from open(path, errors="replace")


def gate_lines(paths):
    """{boot: {gate, unarmed, none, births, why, _line}} — the LAST gate line per boot wins."""
    out = collections.OrderedDict()
    for line in _lines(paths):
        if not line.startswith("APPS_BOOT_GATE "):
            continue
        kv = dict(re.findall(r"(\w+)=(\S*)", line[15:]))
        if kv.get("boot"):
            kv["_line"] = line.rstrip("\n")
            out.pop(kv["boot"], None)  # re-insert: the order is the newest attempt's
            out[kv["boot"]] = kv
    return out


def boots_of(paths):
    """{boot: [app, …]} for every guest row that RAN in a boot (BOOT_FAIL rows never reached the hook)."""
    out = collections.OrderedDict()
    for line in _lines(paths):
        if not line.startswith("APPRES "):
            continue
        kv = dict(KV_ROW.findall(line[7:].strip()))
        if kv.get("verdict") == "BOOT_FAIL" or not kv.get("boot"):
            continue
        apps = out.setdefault(kv["boot"], [])
        if kv.get("app") not in apps:
            apps.append(kv.get("app"))
    return out


def verdict(paths):
    """(head, bad lines, lane) over the given guest.res files."""
    gates = gate_lines(paths)
    ran = boots_of(paths)
    new_format = bool(gates) or any("rc_none=" in line for line in _lines(paths))
    cnt = collections.Counter()
    bad = []
    for boot in list(gates) + [b for b in ran if b not in gates]:
        g = gates.get(boot)
        v = g.get("gate", "UNMEASURED") if g else "UNMEASURED"
        cnt[v] += 1
        if v != "PASS":
            why = g.get("why", "") if g else "no APPS_BOOT_GATE line for a boot that ran apps (its gate never ran)"
            apps = ran.get(boot, [])
            shown = " ".join(apps[:6]) + (f" …+{len(apps) - 6}" if len(apps) > 6 else "")
            bad.append(f"  ⊘ {boot} gate={v} unarmed={(g or {}).get('unarmed', '-')} none={(g or {}).get('none', '-')} "
                       f"births={(g or {}).get('births', '-')} apps=[{shown}] {why}")
    n = sum(cnt.values())
    if not new_format:
        lane = "UNMEASURED"
    elif cnt["FAIL"] or cnt["UNMEASURED"] or n == 0:
        lane = "FAIL"
    else:
        lane = "PASS"
    head = f"BOOT_GATE boots={n} pass={cnt['PASS']} fail={cnt['FAIL']} unmeasured={cnt['UNMEASURED']} lane={lane}"
    if not new_format:
        head += " (these results predate the gate, 2026-10-03: not a pass)"
        bad = []
    return head, bad, lane


def dir_paths(results_dir):
    """The guest lane's gate files of one results directory: batched, then the isolated re-runs."""
    return [os.path.join(results_dir, "guest.res"), os.path.join(results_dir, "iso", "guest.res")]


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a:
        sys.exit(__doc__)
    paths = a[1:] if a[0] == "--files" else dir_paths(a[0])
    head, bad, lane = verdict(paths)
    print(head)
    for b in bad:
        print(b)
    sys.exit(0 if lane == "PASS" else 3)
