"""Read-only host telemetry; GPU calls only after the parent acquires its lease."""

import argparse
import json
from pathlib import Path
import signal
import subprocess
import time

FIELDS = [
    "utilization.gpu",
    "utilization.memory",
    "clocks.sm",
    "clocks.mem",
    "clocks.gr",
    "pstate",
    "power.draw",
    "temperature.gpu",
    "clocks_throttle_reasons.active",
    "memory.used",
    "pcie.link.gen.current",
    "pcie.link.width.current",
]
running = True


def stop(*_args):
    global running
    running = False


def sample(output, seconds, qpid=0):
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    start = time.monotonic()
    target = start
    slow = start
    with Path(output).open("x") as f:
        while running and time.monotonic() - start < seconds:
            before = {"mono_ns": time.monotonic_ns(), "real_ns": time.time_ns()}
            result = subprocess.run(
                [
                    "nvidia-smi",
                    "--query-gpu=" + ",".join(FIELDS),
                    "--format=csv,noheader,nounits",
                    "-i",
                    "0",
                ],
                capture_output=True,
                text=True,
                timeout=5,
            )
            row = {
                "before": before,
                "after": {"mono_ns": time.monotonic_ns(), "real_ns": time.time_ns()},
                "fields": FIELDS,
                "rc": result.returncode,
                "csv": result.stdout.strip(),
                "stderr": result.stderr.strip(),
            }
            if time.monotonic() >= slow:
                row["pressure"] = {
                    k: Path("/proc/pressure/" + k).read_text() for k in ["cpu", "memory", "io"]
                }
                row["procstat"] = Path("/proc/stat").read_text().splitlines()[0]
                if qpid:
                    threads = {}
                    for path in Path(f"/proc/{qpid}/task").glob("*/schedstat"):
                        try:
                            threads[path.parent.name] = {
                                "comm": (path.parent / "comm").read_text().strip(),
                                "schedstat": path.read_text().strip(),
                                "stat": (path.parent / "stat").read_text().strip(),
                            }
                        except FileNotFoundError:
                            pass
                    row["threads"] = threads
                slow += 1
            f.write(json.dumps(row) + "\n")
            f.flush()
            target += 0.1
            # Skip missed deadlines rather than issuing a catch-up query burst.
            while target < time.monotonic():
                target += 0.1
            time.sleep(max(0, target - time.monotonic()))
            if result.returncode:
                raise RuntimeError("nvidia-smi query failed: " + result.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("output")
    parser.add_argument("--seconds", type=float, default=3600)
    parser.add_argument("--qpid", type=int, default=0)
    args = parser.parse_args()
    sample(args.output, args.seconds, args.qpid)
