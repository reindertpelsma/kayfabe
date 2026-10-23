"""Owner17:00 lease-scoped saved signatures: OBSERVATION, never PASS or attestation."""

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import windows_observation as old

G = old.G
Refused = old.Refused
MAX_RECORDS = 8
MODE = "bw3-saved-xid32-observation-v1"
CHANNELS = frozenset((0x14, 0x1000000B))
aware = old.aware


def kernel_time(line):
    return dt.datetime.strptime(line.split()[0], "%Y-%m-%dT%H:%M:%S.%f%z")


def scope_enabled(phase, permit, authority, now):
    if not permit or permit.get("arm") != phase.get("arm"):
        return False
    if (
        authority.get("mode") != MODE
        or permit.get("mode") != MODE
        or authority.get("observation_only") is not True
        or authority.get("attestation_allowed") is not False
        or authority.get("exact_channels") != ["0x14", "0x1000000b"]
    ):
        raise Refused("not the separately authorized saved-signature observation mode")
    return old.scope_enabled(phase, {**permit, "mode": old.MODE}, authority, now)


def judge_observation(lines, *, phase, permit, authority, now):
    if not scope_enabled(phase, permit, authority, now):
        raise Refused("no saved-signature observation permission")
    if len(lines) > MAX_RECORDS:
        raise Refused("more than eight Xid records in this arm")
    records = []
    for line in lines:
        match = G.xid_policy.XID.search(line) if line.count("NVRM: Xid") == 1 else None
        if match is None:
            raise Refused("malformed or non-QEMU Xid record")
        code, pid, channel = int(match[1]), int(match[2]), int(match[3], 16)
        if code != 32 or pid != phase["pid"] or match[4] != "00200000":
            raise Refused("Xid code, owned pid or intr differs from observation signature")
        if channel not in CHANNELS:
            raise Refused("Xid channel differs from the exact saved observation pair")
        if f"NVRM: Xid (PCI:{phase['pci']}):" not in line:
            raise Refused("observation fault belongs to another GPU")
        stamp = kernel_time(line)
        if not aware(permit["armed_utc"]) <= stamp <= now:
            raise Refused("fault timestamp is outside the enabled observation arm")
        records.append(
            {
                "code": code,
                "pid": pid,
                "channel": hex(channel),
                "intr": match[4],
                "timestamp_utc": stamp.isoformat(),
                "line": line,
                "mode": MODE,
                "verdict": "OBSERVATION",
                "pass": False,
                "attestation": False,
            }
        )
    return records


def context(record, rows, dmesg):
    stamp = aware(record["timestamp_utc"])
    before, after = stamp - dt.timedelta(seconds=40), stamp + dt.timedelta(seconds=10)
    kernel = [r["line"] for r in rows if before <= kernel_time(r["line"]) <= after]
    raw = record["line"].split("kernel: ", 1)[1]
    lines = dmesg.splitlines()
    anchors = [i for i, line in enumerate(lines) if raw in line]
    selected = set()
    for index in anchors:
        selected.update(range(max(0, index - 100), min(len(lines), index + 101)))
    if not anchors:
        selected.update(range(max(0, len(lines) - 200), len(lines)))
    return {
        "raw_xid": record["line"],
        "kernel_context": kernel,
        "kernel_window_seconds": [-40, 10],
        "dmesg_context": [lines[i] for i in sorted(selected)],
        "dmesg_anchor_count": len(anchors),
        "dmesg_snapshot_sha256": hashlib.sha256(dmesg.encode()).hexdigest(),
        "mode": MODE,
        "verdict": "OBSERVATION",
        "attestation": False,
        "pass": False,
    }


def dmesg_snapshot():
    child = subprocess.Popen(
        ["timeout", "3", "dmesg", "--time-format=iso", "--color=never"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        raw = child.stdout.read(4 * 1024 * 1024 + 1)
        if len(raw) > 4 * 1024 * 1024 or child.wait(timeout=3) != 0:
            raise ValueError("dmesg context unavailable or exceeds evidence bound")
        return raw.decode("utf-8", "strict")
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=3)


def retain_context(receipt_path, phase, records, rows):
    root = Path(receipt_path).parent / "arms" / phase["arm"]
    root.mkdir(parents=True, exist_ok=True)
    prefix = Path(receipt_path).stem
    contexts = []
    for index, record in enumerate(records):
        path = root / f"{prefix}_dmesg_{index}.txt"
        if not path.exists():
            path.write_text(dmesg_snapshot())
        item = context(record, rows, path.read_text())
        item["full_dmesg_snapshot"] = path.name
        contexts.append(item)
    G.atomic_json(
        root / f"{prefix}_observation_context.json",
        {"mode": MODE, "verdict": "OBSERVATION", "attestation": False, "records": contexts},
    )


def watch(receipt_path, root, phase_path, fatal_path, authority_path, permit_path):
    if root <= 1 or os.getpgid(root) != root or os.getpgrp() != root:
        raise Refused("observation watcher is outside its owned controller group")
    receipt = json.loads(G.N.bounded(receipt_path))
    authority = json.loads(G.N.bounded(authority_path))
    accepted = tuple(receipt["approved_native_xid_lines"])
    initial = old.kernel_rows()
    if G.N.xid_lines("\n".join(row["line"] for row in initial)) != accepted:
        raise Refused("observation startup native/kernel history differs")
    cursors = [row["cursor"] for row in initial]
    old.check_host_health(receipt["pci"])
    phase_key, owned_phase, phase_lines = None, None, ()
    observations = {}
    G.atomic_json(receipt_path + ".ready.json", {"owned_pgid": root, "mode": MODE})
    while True:
        try:
            if Path(fatal_path).exists():
                raise Refused("prior fatal marker")
            phase = json.loads(G.N.bounded(phase_path)) if Path(phase_path).exists() else {}
            key = (phase.get("arm"), phase.get("pid"))
            if key != phase_key:
                if key[0] is not None:
                    phase["expected_pci"] = receipt["pci"]
                    G.validate_phase_owner(phase)
                    owned_phase = dict(phase)
                phase_key, phase_lines = key, ()
            rows = old.kernel_rows()
            new_rows = new_old.kernel_rows(rows, cursors)
            journal = "\n".join(row["line"] for row in rows)
            lines = G.N.xid_lines(journal)
            if lines[: len(accepted)] != accepted:
                raise Refused("frozen native/arm Xid history lost or replaced")
            new_xids = lines[len(accepted) :]
            permit = json.loads(G.N.bounded(permit_path)) if Path(permit_path).exists() else {}
            now = dt.datetime.now(dt.timezone.utc)
            enabled = scope_enabled(phase, permit, authority, now)
            if enabled:
                phase["expected_pci"] = receipt["pci"]
                if new_xids:
                    G.validate_phase_owner(phase)
                previous_lines = tuple(row["line"] for row in observations.get(key[0], []))
                records = judge_observation(
                    previous_lines + new_xids,
                    phase=phase,
                    permit=permit,
                    authority=authority,
                    now=now,
                )
                observations[key[0]] = records
            else:
                G.evaluate_snapshot(journal, accepted, phase_lines, *key, G.phase_init_time(phase))
            if enabled and records:
                try:
                    retain_context(receipt_path, phase, records, rows)
                except Exception as capture_error:
                    print(
                        "EVIDENCE_CONTEXT_UNAVAILABLE " + type(capture_error).__name__, flush=True
                    )
            old.check_kernel_errors(new_rows, new_xids if enabled else ())
            health = old.check_host_health(receipt["pci"])
            phase_lines += new_xids
            accepted = lines
            cursors = [row["cursor"] for row in rows]
            G.atomic_json(
                receipt_path + ".accepted.json",
                {
                    "approved_native_xid_lines": accepted,
                    "phase": key,
                    "observations": observations,
                    "health": health,
                    "observed_utc": now.isoformat(),
                    "mode": MODE,
                    "record_limit_per_arm": MAX_RECORDS,
                    "verdict": "OBSERVATION",
                    "attestation": False,
                    "pass": False,
                },
            )
        except Exception as error:
            try:
                stopped = G.abort_owned_qemu(owned_phase)
                G.record_fatal(
                    fatal_path,
                    {
                        "STOP": str(error),
                        "mode": MODE,
                        "verdict": "OBSERVATION_STOP",
                        "attestation": False,
                        "phase": phase_key,
                        "owned_pgid": root,
                        "detached_qemu_stopped": stopped,
                        "observations": observations,
                    },
                )
            finally:
                G.N.terminate_owned_tree(root)
            raise
        time.sleep(1)  # Existing bench observer interval; no guest/product timer changes.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("receipt", "pgid", "phase", "fatal", "authority", "permit"):
        parser.add_argument(name)
    args = parser.parse_args()
    watch(args.receipt, int(args.pgid), args.phase, args.fatal, args.authority, args.permit)


if __name__ == "__main__":
    main()
