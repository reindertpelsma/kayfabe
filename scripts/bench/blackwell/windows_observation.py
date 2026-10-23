"""Explicit, expiring per-arm Xid32 observation; the normal phase guard stays unchanged."""

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

import phase_xid_guard as G

Refused = G.N.GuardRefused
MAX_RECORDS = 8
MODE = "windows-on-xid32-observation-v1"
ERROR_TEXT = re.compile(r"\b(error|failed|fatal|fallen|panic|oops)\b|RmInitAdapter", re.I)


def aware(value):
    try:
        result = dt.datetime.fromisoformat(value)
    except (ValueError, TypeError) as error:
        raise Refused("invalid observation timestamp") from error
    if result.tzinfo is None:
        raise Refused("observation timestamp lacks timezone")
    return result


def scope_enabled(phase, permit, authority, now):
    if not permit or permit.get("arm") != phase.get("arm"):
        return False
    if authority.get("version") != 1 or authority.get("record_limit") != MAX_RECORDS:
        raise Refused("invalid observation authority or record ceiling")
    if aware(authority["expires_utc"]) - aware(authority["not_before_utc"]) > dt.timedelta(
        minutes=150
    ):
        raise Refused("observation authority exceeds the 150-minute lease ceiling")
    if not aware(authority["not_before_utc"]) <= now < aware(authority["expires_utc"]):
        raise Refused("observation authority is not current")
    if (
        permit.get("mode") != MODE
        or permit.get("profile") != "on"
        or permit.get("arm") not in authority["arms"]
        or not permit["arm"].startswith("on_")
        or permit.get("lease_id") != authority.get("lease_id")
        or type(permit.get("enable_count")) is not int
        or permit["enable_count"] != 1
    ):
        raise Refused("not the authorized single-enable Windows ON arm")
    for key in ("arm", "pid", "starttime", "exe", "pci"):
        if permit.get(key) != phase.get(key):
            raise Refused("observation permit belongs to another process life")
    if type(phase.get("pid")) is not int or phase["pid"] <= 1:
        raise Refused("invalid owned QEMU pid")
    if not aware(authority["not_before_utc"]) <= aware(permit["armed_utc"]) <= now:
        raise Refused("observation enable time is outside authority")
    return True


def judge_observation(lines, *, phase, permit, authority, now):
    if not scope_enabled(phase, permit, authority, now):
        raise Refused("no observation permission for this arm")
    if len(lines) > MAX_RECORDS:
        raise Refused("more than eight Xid records in this arm")
    records = []
    plain, high = set(), set()
    for line in lines:
        match = G.xid_policy.XID.search(line) if line.count("NVRM: Xid") == 1 else None
        if match is None:
            raise Refused("malformed or non-QEMU Xid record")
        code, pid, channel = int(match[1]), int(match[2]), int(match[3], 16)
        if code != 32 or pid != phase["pid"] or match[4] != "00200000":
            raise Refused("Xid code, owned pid or intr differs from observation permission")
        if f"NVRM: Xid (PCI:{phase['pci']}):" not in line:
            raise Refused("observation fault belongs to another GPU")
        if channel < 0x100:
            plain.add(channel)
        elif channel & ~0xFF == 0x0E000000:
            high.add(channel)
        else:
            raise Refused("Xid channel is outside the observed pair class")
        if len(plain) > 1 or len(high) > 1:
            raise Refused("Xid channel changed outside the one observed pair")
        try:
            stamp = dt.datetime.strptime(line.split()[0], "%Y-%m-%dT%H:%M:%S.%f%z")
        except (ValueError, IndexError) as error:
            raise Refused("unparseable observation fault timestamp") from error
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
            }
        )
    return records


def check_kernel_errors(rows, allowed_lines):
    allowed = set(allowed_lines)
    for row in rows:
        if type(row["priority"]) is not int or not 0 <= row["priority"] <= 7:
            raise Refused("kernel record has invalid priority")
        if row["priority"] <= 3 or ERROR_TEXT.search(row["line"]):
            if row["line"] not in allowed:
                raise Refused("new host kernel error beyond the permitted Xid records")


def new_kernel_rows(rows, previous_cursors):
    current = [row["cursor"] for row in rows]
    if current[: len(previous_cursors)] != list(previous_cursors):
        raise Refused("kernel journal history lost or replaced")
    return rows[len(previous_cursors) :]


def check_host_health(expected_pci, *, runner=subprocess.run):
    try:
        smi = runner(
            ["nvidia-smi", "--query-gpu=pci.bus_id,driver_version", "--format=csv,noheader"],
            capture_output=True,
            text=True,
            timeout=3,
        )
        if smi.returncode != 0 or len(smi.stdout) > 2048:
            raise Refused("nvidia-smi host health failed")
        rows = smi.stdout.strip().splitlines()
        if len(rows) != 1:
            raise Refused("host health requires exactly the owned GPU")
        bus, driver = [field.strip() for field in rows[0].split(",")]
        domain, busno, device = bus.split(":")
        pci = f"{int(domain, 16):04x}:{busno}:{device.split('.')[0]}"
        if pci != expected_pci or driver != "595.91.07":
            raise Refused("host health GPU/driver changed")
        cuda = runner(
            [sys.executable, "-I", str(Path(__file__).with_name("cuda_health_probe.py"))],
            capture_output=True,
            text=True,
            timeout=3,
        )
        if cuda.returncode != 0 or len(cuda.stdout) > 2048:
            raise Refused("CUDA host health process failed")
        result = json.loads(cuda.stdout)
        if result.get("cuInit") != 0 or result.get("cuDeviceGetCount") != 0:
            raise Refused("cuInit or cuDeviceGetCount host health failed")
        if type(result.get("devices")) is not int or result["devices"] != 1:
            raise Refused("CUDA host health lost the owned GPU")
        return result
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        raise Refused("host health check failed: " + str(error)) from error


def kernel_rows():
    command = ["timeout", "3", "journalctl", "-k", "-b", "-o", "json", "--no-pager"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        data = proc.stdout.read(G.N.MAX_BYTES + 1)
        if len(data) > G.N.MAX_BYTES or proc.wait(timeout=3) != 0:
            raise Refused("kernel JSON snapshot unreadable or too large")
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=3)
    result = []
    for text in data.decode("utf-8", "strict").splitlines():
        row = json.loads(text)
        stamp = dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc) + dt.timedelta(
            microseconds=int(row["__REALTIME_TIMESTAMP"])
        )
        message, hostname, cursor = row["MESSAGE"], row["_HOSTNAME"], row["__CURSOR"]
        if not all(isinstance(value, str) for value in (message, hostname, cursor)):
            raise Refused("kernel JSON metadata is malformed")
        if len(message) > 16384 or len(cursor) > 1024:
            raise Refused("kernel JSON record exceeds bound")
        line = f"{stamp.strftime('%Y-%m-%dT%H:%M:%S.%f+0000')} {hostname} kernel: {message}"
        result.append({"cursor": cursor, "priority": int(row["PRIORITY"]), "line": line})
    if len(result) < 2:
        raise Refused("empty kernel JSON snapshot")
    return result


def watch(receipt_path, root, phase_path, fatal_path, authority_path, permit_path):
    if root <= 1 or os.getpgid(root) != root or os.getpgrp() != root:
        raise Refused("observation watcher is outside its owned controller group")
    receipt = json.loads(G.N.bounded(receipt_path))
    authority = json.loads(G.N.bounded(authority_path))
    accepted = tuple(receipt["approved_native_xid_lines"])
    initial = kernel_rows()
    if G.N.xid_lines("\n".join(row["line"] for row in initial)) != accepted:
        raise Refused("observation startup native/kernel history differs")
    cursors = [row["cursor"] for row in initial]
    check_host_health(receipt["pci"])
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
            rows = kernel_rows()
            new_rows = new_kernel_rows(rows, cursors)
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
            check_kernel_errors(new_rows, new_xids if enabled else ())
            health = check_host_health(receipt["pci"])
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
                },
            )
        except Exception as error:
            try:
                stopped = G.abort_owned_qemu(owned_phase)
                G.record_fatal(
                    fatal_path,
                    {
                        "STOP": str(error),
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
