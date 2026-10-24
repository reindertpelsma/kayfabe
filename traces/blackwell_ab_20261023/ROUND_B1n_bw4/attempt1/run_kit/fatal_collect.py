"""Box side, after a FATAL stop (owner 2026-10-24 decision 2): one collection boot, logs always.

The collection boot is the existing standard-VGA collect path: the guest disk of the stopped
Windows arm, NO kf3 device, NO GPU work, bounded; it exports dumps, the full event log and the
kf3/QEMU logs into the private capture. A stop outside a Windows arm packs logs only.
Usage: fatal_collect.py   (run by fatal_collect.sh; prints FATAL_COLLECT_* lines)
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys

PROV = Path("/root/prov")
DISKS = Path("/workspace/bw-capc-validation/kfwin/disk")
RUN = re.compile(r"on_[a-z0-9_]{1,37}")


def choose_run(prov, *, disks):
    prov = Path(prov)
    marker = prov / "XID_STOP"
    if not marker.exists():
        raise ValueError("no fatal marker: the collection boot after a stop is not authorized")
    phase = json.loads(marker.read_text()).get("phase") or [None, None]
    arm = phase[0] if phase else None
    if arm is not None:
        # A stop inside a Windows arm collects that arm; inside any other phase (Linux arm,
        # bind, native) there is no Windows guest disk of that phase to collect.
        return arm if type(arm) is str and RUN.fullmatch(arm) and arm in disks else None
    started = sorted(prov.glob("arm_on_*.started"), key=lambda p: p.stat().st_mtime)
    for path in reversed(started):
        run = path.name[len("arm_") : -len(".started")]
        done = prov / f"collect_{run}.log"
        if done.exists() and "COLLECT_ALL_DONE" in done.read_text(errors="replace"):
            return None
        if RUN.fullmatch(run) and run in disks:
            return run
    return None


def preconditions(prov, *, qemu_running, controller_alive):
    if not (Path(prov) / "XID_STOP").exists():
        raise ValueError("no fatal marker")
    if qemu_running:
        raise ValueError("a QEMU is still running; collection waits for none")
    if controller_alive:
        raise ValueError("the controller is still alive; collection is only after a fatal stop")
    return True


def collect_command(run):
    if not RUN.fullmatch(run):
        raise ValueError("invalid arm name")
    return ["timeout", "900", "bash", "/root/sweep2/collect_bw.sh", run]


def xid_delta(before, after):
    if list(after[: len(before)]) != list(before):
        raise ValueError("kernel Xid history changed during the collection boot")
    return list(after[len(before) :])


def kernel_xids():
    text = subprocess.run(
        ["timeout", "10", "journalctl", "-k", "-b", "-o", "short-iso-precise", "--no-pager"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [line for line in text.splitlines() if "NVRM: Xid" in line]


def alive(pid):
    return Path("/proc", str(pid), "exe").exists()


def main():
    controller = int((PROV / "sweep_controller.pid").read_text())
    qemu = subprocess.run(["pgrep", "-x", "qemu-system-x86"], capture_output=True).returncode == 0
    preconditions(PROV, qemu_running=qemu, controller_alive=alive(controller))
    disks = {p.stem for p in DISKS.glob("on_*.qcow2")}
    run = choose_run(PROV, disks=disks)
    print("FATAL_COLLECT_START run=%s" % run, flush=True)
    before = kernel_xids()
    rc = None
    if run is not None:
        rc = subprocess.run(collect_command(run)).returncode
    after = kernel_xids()
    delta = xid_delta(before, after)
    receipt = {
        "run": run,
        "collect_rc": rc,
        "new_xid_during_collection": delta,
        "kf3_device": False,
        "gpu_work": False,
        "standard_vga": run is not None,
        "verdict": "OBSERVATION",
        "attestation": False,
    }
    (PROV / "fatal_collect_receipt.json").write_text(json.dumps(receipt, indent=1) + "\n")
    runs = sorted({p.name[len("arm_") : -len(".started")] for p in PROV.glob("arm_*.started")})
    for name in runs:
        if re.fullmatch(r"[a-z0-9_]{1,40}", name):
            subprocess.run([sys.executable, "-I", "/root/sweep2/capture_pack.py", name])
    print("FATAL_COLLECT_DONE run=%s rc=%s new_xid=%d" % (run, rc, len(delta)), flush=True)
    return 0 if not delta else 3


if __name__ == "__main__":
    os.umask(0o077)
    sys.exit(main())
