"""Bound the VM's transferred bytes before provisioning can consume its allocation."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

DOWNLOAD_BYTES = 12_000_000_000
UPLOAD_BYTES = 1_000_000_000


def exceeded(download, upload):
    if type(download) is not int or type(upload) is not int or min(download, upload) < 0:
        raise ValueError("invalid network byte counter")
    return download >= DOWNLOAD_BYTES or upload >= UPLOAD_BYTES


def monitor(controller):
    fields = subprocess.check_output(["ip", "route", "show", "default"], text=True).split()
    interface = fields[fields.index("dev") + 1]
    if not interface or interface == "lo" or "/" in interface:
        raise ValueError("invalid default network interface")
    root = Path("/sys/class/net", interface, "statistics")
    evidence = Path("/root/sw4/evidence/network.json")
    previous = (0, 0)
    starttime = Path("/proc", str(controller), "stat").read_text().rpartition(")")[2].split()[19]
    while True:
        try:
            current = Path("/proc", str(controller), "stat").read_text()
            if current.rpartition(")")[2].split()[19] != starttime:
                return
            os.kill(controller, 0)
        except (ProcessLookupError, FileNotFoundError):
            return
        rx = int((root / "rx_bytes").read_text())
        tx = int((root / "tx_bytes").read_text())
        stop = exceeded(rx + 200_000_000, tx + 50_000_000)
        stop = stop or rx < previous[0] or tx < previous[1]
        evidence.write_text(
            json.dumps({"interface": interface, "rx_bytes": rx, "tx_bytes": tx, "stopped": stop})
            + "\n"
        )
        if stop:
            Path("/root/sw4/evidence/NETWORK_STOP").write_text("transfer budget stop\n")
            os.killpg(controller, signal.SIGTERM)
            return
        Path("/root/sw4/evidence/network_armed.json").write_text(
            json.dumps({"controller_pid": controller, "starttime": starttime}) + "\n"
        )
        previous = (rx, tx)
        time.sleep(1)


def run_owned(monitor, stop):
    try:
        monitor()
    except (OSError, ValueError, KeyError, IndexError, subprocess.SubprocessError):
        stop()


def main():
    controller = int(sys.argv[1])
    if controller <= 1 or os.getpgid(controller) != controller:
        raise ValueError("controller is not the owned session leader")

    def stop():
        Path("/root/sw4/evidence/NETWORK_STOP").write_text("transfer observer refused\n")
        os.killpg(controller, signal.SIGTERM)

    run_owned(lambda: monitor(controller), stop)


if __name__ == "__main__":
    main()
