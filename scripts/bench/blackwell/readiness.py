"""Bench-only latency calibration. No product, fault-policy or TDR timer changes."""

import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import time


def policy(seconds):
    if type(seconds) not in (int, float) or not math.isfinite(seconds) or seconds <= 0:
        raise ValueError("invalid probe duration")
    factor = min(10, max(4, math.ceil(seconds / 0.5)))
    return {
        "factor": factor,
        "first_query": 10 * factor,
        "steady_query": math.ceil(10 * factor / 4),
        "banner_query": min(100, 20 * factor),
        "handshake": 20 * factor + 2,
        "observer_total": 360,
        "verify_stage": 240,
        "ssh_stage": 600,
        "readiness_total": 900,
    }


class Budget:
    def __init__(self, stage, total, *, clock=time.monotonic):
        self.clock = clock
        self.end = clock() + min(stage, total)

    def timeout(self, requested):
        remaining = self.end - self.clock()
        if remaining <= 0:
            raise TimeoutError("readiness budget exhausted")
        return min(requested, remaining)


def first_query(fetch, budget, requested):
    for attempt in range(2):
        try:
            allowed = budget.timeout(requested)
            if attempt == 0:
                allowed = min(allowed, budget.timeout(float("inf")) / 2)
            text = fetch(allowed)
        except subprocess.TimeoutExpired:
            if attempt == 1:
                raise
            continue
        budget.timeout(requested)  # Never publish a result after the absolute deadline.
        return text
    raise AssertionError("unreachable")


def calibration(path, box, measure):
    path = Path(path)
    if path.exists():
        row = json.loads(path.read_text())
        if row.get("box") != box:
            raise ValueError("calibration belongs to another box")
        if row.get("policy") != policy(row["probe_seconds"]):
            raise ValueError("calibration policy mismatch")
        return row
    seconds = measure()
    row = {"box": box, "probe_seconds": seconds, "policy": policy(seconds)}
    # Exclusive creation: a repeated arm cannot overwrite the box's measurement.
    with path.open("x") as out:
        json.dump(row, out, sort_keys=True)
        out.write("\n")
    return row


def command(argv, timeout):
    env = dict(os.environ, WINVM_SSH_CONNECT_TIMEOUT=str(min(100, math.ceil(timeout))))
    r = subprocess.run(
        argv,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env,
    )
    if r.returncode == 124:
        raise subprocess.TimeoutExpired(argv, timeout)
    if r.returncode:
        raise ValueError("readiness command failed: " + str(r.returncode))
    if len(r.stdout) > 65536:
        raise ValueError("readiness reply byte bound")
    return r.stdout


def boot(phase, path, lease, wrapper):
    import init_observer as O
    import phase_xid_guard as G

    frozen = O.state(phase)
    box = json.loads(Path(lease).read_text())["id"]
    readiness_start = time.monotonic()
    budget = Budget(600, 900)

    def validate():
        current = O.state(phase)
        if any(current.get(k) != frozen.get(k) for k in O.IDENTITY):
            raise ValueError("readiness VM identity changed")
        G.validate_phase_owner(dict(current, expected_pci=current["pci"]))

    def probe():
        validate()
        start = time.monotonic()
        try:
            text = command(["bash", wrapper, "ssh", "echo KF_SSH_UP"], budget.timeout(100))
        finally:
            validate()
        if "KF_SSH_UP" not in text.split():
            raise ValueError("missing SSH readiness marker")
        budget.timeout(100)
        return max(0.001, time.monotonic() - start)

    measured = None
    for _ in range(60):
        try:
            measured = probe()
            break
        except (subprocess.TimeoutExpired, ValueError) as error:
            # Only SSH's unavailable/banner code 255 is retryable while booting.
            if isinstance(error, ValueError) and str(error) != "readiness command failed: 255":
                raise
            budget.timeout(100)
            time.sleep(min(5, budget.timeout(5)))
    if measured is None:
        raise TimeoutError("SSH readiness exhausted")
    row = calibration(path, box, lambda: measured)
    row = dict(row, readiness_end=readiness_start + 900)
    runtime = Path(str(path) + ".arm.json")
    runtime.write_text(json.dumps(row) + "\n")
    print("READINESS_CALIBRATED", json.dumps(row), flush=True)


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="mode", required=True)
    b = sub.add_parser("boot")
    for name in ("phase", "calibration", "lease", "wrapper"):
        b.add_argument(name)
    e = sub.add_parser("value")
    e.add_argument("calibration")
    e.add_argument("key", choices=list(policy(1)))
    r = sub.add_parser("run")
    r.add_argument("calibration")
    r.add_argument("stage", choices=["verify_stage"])
    r.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args()
    if a.mode == "boot":
        boot(a.phase, a.calibration, a.lease, a.wrapper)
        return
    row = json.loads(Path(a.calibration).read_text())
    row = calibration(a.calibration, row["box"], lambda: None)
    if a.mode == "value":
        print(row["policy"][a.key])
        return
    runtime = json.loads(Path(a.calibration + ".arm.json").read_text())
    remaining = runtime["readiness_end"] - time.monotonic()
    budget = Budget(row["policy"][a.stage], remaining)
    argv = a.command[1:] if a.command[:1] == ["--"] else a.command
    print(first_query(lambda t: command(argv, t), budget, 120 * row["policy"]["factor"]), end="")


if __name__ == "__main__":
    main()
