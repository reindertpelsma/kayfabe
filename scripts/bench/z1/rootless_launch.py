"""Rootless QEMU lifetime and per-process capability/device audit.

Adapted from the 2026-10-16 audit's as_user.sh and watch_caps.sh. This observer
runs as the same unprivileged uid as QEMU; host kernel-log observation is separate.
"""

import grp
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from host_guard import Refused, identity, preflight, status
from host_guard import check_privilege


def main():
    own = status(os.getpid())
    check_privilege(
        os.geteuid(),
        int(own["CapEff"], 16),
        int(own["CapPrm"], 16),
        set(os.getgroups()),
        grp.getgrnam("kvm").gr_gid,
        os.getegid(),
    )
    resources = preflight(int(os.environ.get("NVKVM_RAM_MB", "2048")))
    if sys.argv[1:] == ["--preflight"]:
        print(json.dumps({"rootless_preflight": resources, "id": os.geteuid()}))
        return 0
    binary = Path(os.environ["KF_REAL_QEMU"]).resolve(strict=True)
    digest = hashlib.file_digest(binary.open("rb"), "sha256").hexdigest()
    if digest != os.environ["KF_QEMU_SHA256"]:
        raise Refused("QEMU binary hash differs from preregistered asset")
    out = Path(os.environ["KF_Z1_EVIDENCE"])
    out.mkdir(parents=True, exist_ok=True)
    child = subprocess.Popen([str(binary), *sys.argv[1:]])
    record = {
        "pid": child.pid,
        "starttime": identity(child.pid),
        "sha256": digest,
        "argv": sys.argv[1:],
        "resources": resources,
        "samples": [],
        "qlog": str(Path("/proc/self/fd/1").resolve()),
        "parent": os.getpid(),
        "observed_devices": [],
    }
    receipt = out / f"qemu-{child.pid}.json"
    current = out / "current.json"

    def stop(signum, _frame):
        if child.poll() is None:
            child.send_signal(signum)

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        while child.poll() is None:
            try:
                sample = status(child.pid)
                if (
                    int(sample["Uid"].split()[1]) != os.geteuid()
                    or int(sample["CapEff"], 16)
                    or int(sample["CapPrm"], 16)
                ):
                    raise Refused("QEMU privilege differs from unprivileged launcher")
                nodes = set()
                for fd in Path(f"/proc/{child.pid}/fd").iterdir():
                    try:
                        target = str(fd.resolve(strict=True))
                        if target.startswith("/dev/"):
                            nodes.add(target)
                    except FileNotFoundError:
                        pass
                record["observed_devices"] = sorted(set(record["observed_devices"]) | nodes)
                if len(record["samples"]) < 3:
                    record["samples"].append(
                        {
                            "time": time.time(),
                            "status": {
                                key: sample[key].strip()
                                for key in (
                                    "Uid",
                                    "Gid",
                                    "Groups",
                                    "CapEff",
                                    "CapPrm",
                                    "CapBnd",
                                    "NoNewPrivs",
                                    "VmLck",
                                    "VmPin",
                                )
                                if key in sample
                            },
                            "devices": sorted(nodes),
                        }
                    )
                temp_receipt = out / f"qemu-{child.pid}.tmp"
                temp_receipt.write_text(json.dumps(record, indent=2) + "\n")
                temp_receipt.replace(receipt)
                temp = out / "current.tmp"
                temp.write_text(json.dumps(record) + "\n")
                temp.replace(current)
            except (FileNotFoundError, ProcessLookupError):
                if child.poll() is None:
                    continue
            time.sleep(0.1)
        record["rc"] = child.wait()
        record["ended"] = time.time()
        temp_receipt = out / f"qemu-{child.pid}.tmp"
        temp_receipt.write_text(json.dumps(record, indent=2) + "\n")
        temp_receipt.replace(receipt)
        temp = out / "current.tmp"
        temp.write_text(json.dumps(record) + "\n")
        temp.replace(current)
        ack = os.environ.get("KF_Z1_HOST_ACK_DIR")
        if ack:
            deadline = time.monotonic() + 60
            while not (Path(ack) / f"{child.pid}.pass").exists():
                if (Path(ack) / "STOP").exists() or time.monotonic() >= deadline:
                    raise Refused("host observer refused or did not acknowledge the arm")
                time.sleep(0.1)
        return record["rc"]
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (Refused, OSError, KeyError, ValueError) as error:
        print(f"ROOTLESS_REFUSED: {error}", file=sys.stderr)
        sys.exit(2)
