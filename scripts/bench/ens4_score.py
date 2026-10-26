#!/usr/bin/env python3
"""Offline ENS4 Linux gate. A client PASS alone is never a gate PASS."""
import argparse
import csv
import json
import re
from pathlib import Path


def integer(value, limit=(1 << 64) - 1):
    if not isinstance(value, str) or not value.isascii() or not value.isdecimal():
        raise ValueError("expected bounded unsigned decimal")
    number = int(value)
    if number > limit:
        raise ValueError("numeric field exceeds bound")
    return number


def one(pattern, text):
    values = re.findall(pattern, text)
    if len(values) != 1:
        raise ValueError("missing or ambiguous diagnostic identity/result")
    return values[0]


def score(client, qemu, rows):
    try:
        if not 10001 <= len(rows) <= 1_000_000:
            raise ValueError("sample count outside bounded calibration/churn population")
        clean, tested = [], []
        in_churn = False
        for index, row in enumerate(rows):
            if integer(row["index"], 1_000_000) != index:
                raise ValueError("missing/reordered sample")
            phase = row["phase"]
            receipt = row["engine_receipt"]
            if receipt not in ("true", "false"):
                raise ValueError("invalid engine receipt flag")
            ns = integer(row["latency_ns"]) if row["latency_ns"] else None
            if (receipt == "true") != (ns is not None) or ns == 0:
                raise ValueError("inconsistent completion receipt")
            if phase == "calibration" and not in_churn and ns is not None:
                clean.append(ns)
            elif phase == "churn":
                in_churn = True
                tested.append(ns)
            else:
                raise ValueError("missing calibration receipt or invalid phase order")
        if len(clean) != 10000 or not tested:
            raise ValueError("need exactly10000 clean samples and overlapping churn packets")
        p999 = sorted(clean)[(999 * len(clean) + 999) // 1000 - 1]
        bound = p999 * 4
        frozen = one(r"ENS4_CALIB n=(\d+) p999_ns=(\d+) k=4 bound_ns=(\d+) frozen=true", client)
        if tuple(map(int, frozen)) != (len(clean), p999, bound):
            raise ValueError("calibration log disagrees with complete CSV")
        owner, channel = one(r"ENS4_CHANNEL client=(0x[0-9a-f]+) channel=(0x[0-9a-f]+)", client)
        churn_owner, vas = one(r"ENS4_CHURN client=(0x[0-9a-f]+) vaspace=(0x[0-9a-f]+)", client)
        if owner != churn_owner:
            raise ValueError("churn belongs to a different client")
        for value in [owner, channel, vas]:
            if int(value, 16) >= 1 << 32:
                raise ValueError("RM identity exceeds u32")
        route = (rf"Windows USER ring chan {owner}:{channel} BORN USER "
                 r"private-read-only-ring token 0x[0-9a-f]+ host (0x[0-9a-f]+);")
        token = one(route, qemu)
        space = (int(owner, 16) << 32) | int(vas, 16)
        applies = re.findall(r"ENS4 APPLY space=(0x[0-9a-f]+) walk=\d+ runs=(\d+) ", qemu)
        if not any(int(key, 16) == space and int(runs) >= 1000 for key, runs in applies):
            raise ValueError("no qualifying large apply in the named churn space")
        hold = re.search(rf"GATELOG HOLDTOP token={token} .*?holds=([1-9]\d*) ", qemu)
        if not hold or "GATELOG END #" not in qemu:
            raise ValueError("missing named-channel hold or terminated gate dump")
        packets, rounds = one(r"ENS4_LINUX packets=(\d+) rounds=(\d+) maps_per_round=4174", client)
        if int(packets) != len(tested) or not 1 <= int(rounds) <= 32:
            raise ValueError("inadequate churn or sample-count mismatch")
        failed = [i for i, ns in enumerate(tested) if ns is None or ns > bound]
        verdict = "FAIL" if failed else "PASS"
        claimed = one(r"ENS4_CLIENT_VERDICT=(PASS|FAIL)", client)
        rc = one(r"ENS4_CLIENT_DONE rc=([012])", client)
        if claimed != verdict or int(rc) != bool(failed) or "ENS4_CLIENT_INVALID=" in client:
            raise ValueError("client endpoint disagrees with complete receipt CSV")
        return dict(verdict=verdict, p999_ns=p999, k=4, bound_ns=bound,
                    worst_completion_ns=max((n for n in tested if n is not None), default=None),
                    first_fail=failed[0] if failed else None, packets=len(tested),
                    rounds=int(rounds), host_token=token, receipt="owned_CE_and_WFI_fence",
                    attribution="boot-level exposure; not matched Windows packet causality")
    except (ValueError, KeyError, TypeError) as error:
        return dict(verdict="INVALID", why=str(error))


def bounded_text(path, limit):
    path = Path(path)
    if path.stat().st_size > limit:
        raise ValueError(f"input exceeds size cap: {path.name}")
    return path.read_text()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-log", required=True)
    parser.add_argument("--qemu-log", required=True)
    parser.add_argument("--csv", required=True)
    args = parser.parse_args()
    try:
        client = bounded_text(args.client_log, 16 << 20)
        qemu = bounded_text(args.qemu_log, 256 << 20)
        content = bounded_text(args.csv, 64 << 20)
        rows = list(csv.DictReader(content.splitlines()))
        result = score(client, qemu, rows)
    except (OSError, ValueError, csv.Error) as error:
        result = dict(verdict="INVALID", why=str(error))
    print(json.dumps(result, sort_keys=True))
    print(f"ENS4_GATE_VERDICT={result['verdict']}")
    return {"PASS": 0, "FAIL": 1, "INVALID": 2}[result["verdict"]]


if __name__ == "__main__":
    raise SystemExit(main())
