#!/usr/bin/env python3
"""Isolated native read-only storage phase probe; no guest or product fallback."""
import argparse
import datetime as dt
import fcntl
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("revision")
    parser.add_argument("result", type=Path)
    args = parser.parse_args()
    lock = open("/tmp/kayfabe-fastguest.lock", "a")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    if subprocess.run(["pgrep", "-x", "qemu-system-x86"], capture_output=True).returncode != 1:
        raise RuntimeError("QEMU active or process check failed")
    if Path("/sys/kernel/iommu_groups/11/type").read_text().strip() != "DMA-FQ":
        raise RuntimeError("IOMMU group 11 is not DMA-FQ")
    if Path("/tmp/kf-stop-winprod").exists():
        raise RuntimeError("Windows stop marker present")
    if not re.fullmatch(r"[0-9a-f]{40}", args.revision):
        raise RuntimeError("Full source revision required")
    args.result.mkdir(parents=True, exist_ok=False)
    start = now()
    (args.result / "START").write_text(start + "\n")
    source = dict(revision=args.revision, binary=str(args.binary),
                  binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                  uid=65534, gid=65534, scope="native phased storage/control barrier; no VM")
    (args.result / "source.json").write_text(json.dumps(source, indent=2) + "\n")
    with (args.result / "probe.log").open("wb") as log:
        proc = subprocess.Popen([str(args.binary), "storage-control-barrier-phased"],
                                stdout=log, stderr=subprocess.STDOUT,
                                user=65534, group=65534, extra_groups=[],
                                env={"PATH": "/usr/bin:/bin", "LANG": "C"})
        print("USER_STORAGE_PHASE_START", start, "pid", proc.pid, flush=True)
        killed = False
        try:
            rc = proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            # Bench observation deadline only. No work/reuse credit is granted.
            killed = True
            proc.kill()
            rc = proc.wait()
    time.sleep(2)
    end = now()
    journal = subprocess.run(["journalctl", "-k", "--since", start, "--until", end,
                              "--no-pager", "-o", "short-iso"], capture_output=True, text=True)
    xids = [line for line in journal.stdout.splitlines() if re.search(r"NVRM: Xid", line)]
    (args.result / "xid.log").write_text("\n".join(xids) + ("\n" if xids else ""))
    text = (args.result / "probe.log").read_text()
    phases = re.search(r"USER_STORAGE_PHASES prepare_us=(\d+) clear_steps=(\d+) max_clear_us=(\d+) map_us=(\d+)", text)
    timings = dict(zip(["prepare_us", "clear_steps", "max_clear_us", "map_us"],
                       map(int, phases.groups()))) if phases else None
    cleanup = "USER_STORAGE_CLEANUP result=Ok(()) released=true" in text
    passed = (rc == 0 and not killed and journal.returncode == 0 and not xids
              and "USER_RING_PROBE=PASS" in text and cleanup and timings is not None
              and timings["clear_steps"] == 256
              and max(timings[k] for k in ("prepare_us", "max_clear_us", "map_us")) <= 12_000)
    verdict = dict(verdict="PASS" if passed else "FAIL", start_utc=start, end_utc=end,
                   pid=proc.pid, rc=rc, killed=killed, journal_query_exit=journal.returncode,
                   xid_records=len(xids), cleanup_ok=cleanup, phases=timings)
    (args.result / "verdict.json").write_text(json.dumps(verdict, indent=2) + "\n")
    (args.result / "EXIT").write_text(end + " verdict=" + verdict["verdict"] + "\n")
    print("USER_STORAGE_PHASE_EXIT", json.dumps(verdict), flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
