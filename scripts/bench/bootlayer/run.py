#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Own one bounded capture VM under run.sh's shared GPU lock.

run.sh LANE OUTPUT REVISION SECONDS [--store-reader EXECUTABLE] -- QEMU ARGS...
The output must be a new directory below /data/LANE. Supply a disposable guest,
its UUID, a kf3 id=kf0 device, broker settings, RAM and disks in QEMU ARGS.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import signal
import subprocess
import sys
import time
import uuid


def stop_child(child):
    if child is not None and child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=10)


def main():
    try:
        split = sys.argv.index("--")
    except ValueError:
        raise SystemExit(__doc__)
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("lane")
    p.add_argument("output", type=Path)
    p.add_argument("revision")
    p.add_argument("seconds", type=float)
    p.add_argument("--store-reader")
    a = p.parse_args(sys.argv[1:split])
    command = sys.argv[split + 1:]
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,48}", a.lane):
        p.error("invalid lane directory name")
    out = a.output.resolve()
    root = Path("/data") / a.lane
    if not out.is_relative_to(root) or out == root:
        p.error("OUTPUT must be a new directory below /data/LANE")
    if not re.fullmatch(r"[0-9a-f]{40}", a.revision):
        p.error("REVISION must identify the exact 40-character source commit")
    if not (0 < a.seconds <= 900):
        p.error("SECONDS must be in (0,900]")
    if not command or not Path(command[0]).name.startswith("qemu-system-"):
        p.error("supply direct QEMU argv, not a launcher or shell expression")
    try:
        vm_uuid = str(uuid.UUID(command[command.index("-uuid") + 1]))
    except (ValueError, IndexError):
        p.error("QEMU argv must supply a stable -uuid (OWNER_RULINGS section V)")
    if any(x in command for x in ("-daemonize", "-no-reboot", "-incoming")):
        p.error("capture requires an owned foreground QEMU and reset within the same process")
    if not any("kf3-gpu" in arg and "id=kf0" in arg for arg in command):
        p.error("QEMU argv must name the captured kf3-gpu,id=kf0 device")
    if a.store_reader and not Path(a.store_reader).is_file():
        p.error("store diagnostic adapter does not exist")
    out.mkdir(parents=True, exist_ok=False)
    binary = Path(command[0]).resolve(strict=True)
    manifest = {"revision": a.revision, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "binary": str(binary), "start_ns": time.time_ns(), "capture_seconds": a.seconds,
                "hardware_verdict": "NOT_JUDGED", "same_process_reboot": True, "uuid": vm_uuid}
    (out / "run.json").write_text(json.dumps(manifest, indent=2) + "\n")
    here = Path(__file__).resolve().parent
    cap = [sys.executable, str(here / "capture.py"), str(out), "--revision", a.revision,
           "--seconds", str(a.seconds)]
    if a.store_reader:
        cap += ["--store-reader", a.store_reader]
    command += ["-S", "-qmp", f"unix:{out}/capture.qmp,server=on,wait=off",
                "-qmp", f"unix:{out}/control.qmp,server=on,wait=off", "-action", "reboot=reset"]
    vm = capture = None

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        with (out / "capture.log").open("x") as caplog, (out / "qemu.log").open("x") as qlog:
            capture = subprocess.Popen(cap, stdout=caplog, stderr=subprocess.STDOUT)
            vm = subprocess.Popen(command, stdout=qlog, stderr=subprocess.STDOUT)
            (out / "qemu.pid").write_text(str(vm.pid) + "\n")
            result = capture.wait(timeout=a.seconds + 180)
            manifest["capture_exit"] = result
            return result
    except subprocess.TimeoutExpired:
        manifest["error"] = "CAPTURE_DEADLINE_EXCEEDED"
        return 124
    except KeyboardInterrupt:
        manifest["interrupted"] = True
        return 130
    finally:
        # These are Popen children started here, never a PID found by name.
        stop_child(capture)
        stop_child(vm)
        manifest["end_ns"] = time.time_ns()
        manifest["children_reaped"] = all(c is None or c.poll() is not None for c in (capture, vm))
        (out / "run.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    raise SystemExit(main())
