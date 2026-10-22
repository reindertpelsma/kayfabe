#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Collect an owned tier-3 observation, never authorize a guest Xid baseline.

CLIENT is the measured field. Every other source-fault field, process identity,
and record multiplicity stays exact. The production admission remains separate.
"""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import re

SPEC = importlib.util.spec_from_file_location(
    "native_xid_guard", Path(__file__).resolve().parents[1] / "native_xid_guard.py")
G = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(G)


class MeasurementRefused(ValueError):
    """The source-correlated calibration record is incomplete or unmatched."""


def measure(before, journal, defer, missing, pids):
    if G.xid_lines(before):
        raise MeasurementRefused("preexisting Xid: no clean measurement interval")
    if set(pids) != {"defer", "missing"} or any(type(x) is not int or x <= 1 for x in pids.values()):
        raise MeasurementRefused("missing owned native process identities")
    try:
        expected = G.expected_faults(defer, missing)
        records = [G._guest_record(line) for line in G.xid_lines(journal)]
    except G.GuardRefused as error:
        raise MeasurementRefused(str(error)) from error
    if len(expected) != 5:
        raise MeasurementRefused("native proof must name exactly five faults")
    # Drop only the CE CLIENT, which is the observation sought by this probe.
    expected_shapes = [row[:6] + row[7:] for row in expected]
    shapes = []
    copy_faults = []
    for record in records:
        if record["intr"] != "00000000":
            raise MeasurementRefused("unexpected interrupt field")
        copy = record["engine"] == "CE0"
        if int(record["pid"]) != pids["defer" if copy else "missing"]:
            raise MeasurementRefused("fault process differs from owned native child")
        if copy:
            if re.fullmatch(r"HUBCLIENT_(?:CE|HSCE)[0-9]+", record["client"]) is None:
                raise MeasurementRefused("unrecognized CE fault client")
        elif record["client"] != "HUBCLIENT_ESC":
            raise MeasurementRefused("unexpected GR negative-control client")
        channel = int(record["channel"], 16)
        va = int(record["va"].replace("_", ""), 16)
        shapes.append((int(record["code"]), channel, va, record["fault"],
                       record["access"], record["engine"], record["name"]))
        if copy:
            copy_faults.append({"pid": int(record["pid"]), "channel": channel,
                                "va": hex(va), "client": record["client"]})
    if Counter(shapes) != Counter(expected_shapes):
        raise MeasurementRefused("fault shapes or multiplicity differ from exact source controls")
    return {"kind": "MEASURED", "scope": "owned native route observations only",
            "approved_for_guest": False, "xid_count": len(records),
            "copy_faults": copy_faults,
            "guard_policy": "unchanged; this result is not an admission baseline"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before")
    parser.add_argument("after")
    parser.add_argument("defer")
    parser.add_argument("missing")
    parser.add_argument("pids")
    args = parser.parse_args()
    result = measure(G.bounded(args.before), G.bounded(args.after),
                     G.bounded(args.defer), G.bounded(args.missing),
                     json.loads(G.bounded(args.pids)))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
