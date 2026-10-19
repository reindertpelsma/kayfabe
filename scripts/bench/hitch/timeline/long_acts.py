#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Inventory printed acts >20 ms. Missing identities/timestamps stay UNKNOWN.

An act timer measures its whole closure, not an individual RM call. Quiet and
requeued acts can omit a line, so this is an observed inventory, not a census.
Only an explicit act_end_mono_ns event stamp establishes event time. A tailer's
line-arrival clock is deliberately not treated as the act's completion time.
"""
import argparse
import json
import pathlib
import re

ACT = re.compile(r"kf3: act ([^:]+): (.*?) \((\d+) us(?:, off the GSP lock)?\)\s*$")
HANDLES = re.compile(r"\b(0x[0-9a-fA-F]+):(0x[0-9a-fA-F]+)\b")


def parse_lines(lines, *, threshold_us=20_000, vm_start_ns=None):
    """Return observed long acts plus parse coverage; never infer a PID from RM handles."""
    if threshold_us < 0:
        raise ValueError("negative act threshold")
    acts, timed, unparsed = [], 0, 0
    for number, line in enumerate(lines, 1):
        if "kf3: act " not in line:
            continue
        match = ACT.search(line)
        if not match:
            unparsed += 1
            continue
        timed += 1
        name, detail, elapsed = match.groups()
        duration = int(elapsed)
        if duration <= threshold_us:
            continue
        handles = HANDLES.search(detail)
        cls = re.search(r"\bclass (0x[0-9a-fA-F]+)\b", detail)
        quantum = re.search(r"\bSET_TIMESLICE (\d+) us\b", detail)
        mode = re.search(r"\bgroup_mode=(LEGACY|EXPLICIT)\b", detail)
        stamp = re.search(r"\bact_end_mono_ns=(\d+)\b", line)
        ts = int(stamp[1]) if stamp else None
        api = "SET_TIMESLICE" if quantum else "RM_ALLOC" if name == "engine object" else "UNKNOWN"
        acts.append(dict(line=number, act_name=name, api=api, duration_us=duration,
                         requested_timeslice_us=int(quantum[1]) if quantum else None,
                         control_id="0xa06c0103" if quantum else None,
                         object_class=cls[1].lower() if cls else None,
                         guest_client=handles[1].lower() if handles else None,
                         guest_object=handles[2].lower() if handles else None,
                         group_mode=mode[1] if mode else None,
                         executor_thread_name="kf3-chan-act", executor_name_basis="CODE READ",
                         executor_tid=None, guest_waiter_pid=None, guest_waiter_tid=None,
                         ts_ns=ts, vm_uptime_ns=ts-vm_start_ns
                         if ts is not None and vm_start_ns is not None else None,
                         detail=detail))
    return dict(acts=acts, timed_act_lines=timed, unparsed_act_lines=unparsed,
                completeness="observed printed acts only; silent/requeued acts can be absent")


def format_table(result):
    """TSV with explicit unknown fields; safe to inspect or import into a spreadsheet."""
    columns = ("line", "api", "object_class", "guest_client", "guest_object", "group_mode",
               "duration_us", "executor_thread_name", "executor_tid", "guest_waiter_pid",
               "guest_waiter_tid", "vm_uptime_ns")
    rows = ["\t".join(columns)]
    for act in result["acts"]:
        rows.append("\t".join("UNKNOWN" if act[k] is None else str(act[k]) for k in columns))
    return "\n".join(rows) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log")
    parser.add_argument("--vm-start-ns", type=int)
    parser.add_argument("--out", help="optional structured JSON inventory")
    args = parser.parse_args()
    with pathlib.Path(args.log).open(errors="replace") as stream:
        result = parse_lines(stream, vm_start_ns=args.vm_start_ns)
    print(format_table(result), end="")
    if args.out:
        pathlib.Path(args.out).write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
