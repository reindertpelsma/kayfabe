#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded QMP screenshot series; start before QEMU, continue its initial -S pause.

Only the host lane executes this. QMP screendump observes QEMU's console; it does
not prove the broker presented the same image. Keep the broker's own capture and
log as an independent observation when exercising that path.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import queue
import re
import shutil
import threading
import time

from phase_dump import dump, PHASES

# Reuse the repository's existing QMP transport rather than adding a second one.
QMP_PATH = Path(__file__).resolve().parents[1] / "windows/host_tools/runners/boundary-tools/qmp.py"
spec = importlib.util.spec_from_file_location("bootlayer_qmp", QMP_PATH)
qmp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qmp)
TRIGGERS = {
    "fn72": re.compile(r"GSP rpc GspSetSystemInfo\b"),
    "armed": re.compile(r"the console shows (?:head |the ARMED|an ARMED)"),
    "unloaded": re.compile(r"boot framebuffer store .* scrubbed first"),
    "reboot_scrubbed": re.compile(r"NEW OS:.*completion awaited, (\d+) ms"),
}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("out", type=Path)
    p.add_argument("--revision", required=True)
    p.add_argument("--seconds", type=float, default=180)
    p.add_argument("--interval", type=float, default=0.1)
    p.add_argument("--max-bytes", type=int, default=2 * 1024**3)
    p.add_argument("--store-reader")
    p.add_argument("--device", default="kf0")
    a = p.parse_args()
    if not (0 < a.seconds <= 900 and 0.05 <= a.interval <= 1 and 0 < a.max_bytes <= 8 * 1024**3):
        p.error("bounded capture requires seconds<=900, interval 0.05..1, max-bytes<=8GiB")
    out = a.out.resolve()
    frames = out / "frames"
    frames.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    deadline = started + a.seconds
    log_offset = 0
    queued = set()
    work = queue.Queue(maxsize=5)
    finished = []
    errors = []

    def store_worker():
        while True:
            item = work.get()
            if item is None:
                return
            phase, trigger_ns = item
            try:
                result = dump(out, phase, a.store_reader, a.revision)
                result["trigger_ns"] = trigger_ns
                finished.append(result)
            except Exception as error:
                errors.append(f"STORE_{phase}: {error}")
                finished.append({"phase": phase, "status": "UNAVAILABLE"})

    qmp_events = (out / "qmp-events.jsonl").open("x")

    def command(name, args=None):
        message = {"execute": name}
        if args:
            message["arguments"] = args
        qmp.send(stream, message)
        while True:
            reply = qmp.recv(stream)
            if "event" in reply:
                qmp_events.write(json.dumps(reply) + "\n")
                qmp_events.flush()
            else:
                return reply

    worker = threading.Thread(target=store_worker, name="bootlayer-store")
    worker.start()
    sock = stream = None
    status = "INCOMPLETE"
    total = count = 0
    pending = b""
    try:
        while time.monotonic() < min(deadline, started + 60):
            try:
                sock, stream = qmp.qmp_open(str(out / "capture.qmp"), timeout=2)
                break
            except (OSError, EOFError, ValueError, RuntimeError):
                time.sleep(0.05)
        if stream is None:
            raise RuntimeError("QMP_NOT_READY: no initial paused console within capture bound")
        # This phase is synchronous while QEMU is paused: firmware has not drawn yet.
        queued.add("realized")
        finished.append(dump(out, "realized", a.store_reader, a.revision))
        response = command("cont")
        if "error" in response:
            raise RuntimeError("QMP cont refused: " + str(response["error"]))
        next_frame = time.monotonic()
        with (out / "frames.jsonl").open("x") as manifest, (out / "events.jsonl").open("x") as events:
            while time.monotonic() < deadline:
                if shutil.disk_usage(out).free < 4 * 1024**3:
                    raise RuntimeError("DISK_GUARD: less than 4GiB free; capture stopped")
                log = out / "qemu.log"
                if log.exists():
                    with log.open("rb") as f:
                        f.seek(log_offset)
                        pending += f.read(1024 * 1024)
                        log_offset = f.tell()
                    lines = pending.split(b"\n")
                    pending = lines.pop()
                    if len(pending) > 1024 * 1024:
                        raise RuntimeError("LOG_RECORD_TOO_LARGE")
                    for raw in lines:
                        line = raw.decode("utf-8", "replace")
                        for phase, regex in TRIGGERS.items():
                            match = regex.search(line)
                            if match and phase not in queued:
                                observed = time.time_ns()
                                queued.add(phase)
                                events.write(json.dumps({"phase": phase, "observed_ns": observed,
                                                         "line": line[:4096]}) + "\n")
                                events.flush()
                                work.put_nowait((phase, observed))
                                if phase == "reboot_scrubbed":
                                    (out / "new-os.txt").write_text(line + "\n")
                if time.monotonic() < next_frame:
                    time.sleep(min(0.01, next_frame - time.monotonic()))
                    continue
                image = frames / f"{count:06d}.png"
                before = time.time_ns()
                response = command("screendump", {"filename": str(image),
                                       "device": a.device, "head": 0, "format": "png"})
                after = time.time_ns()
                if "error" in response:
                    raise RuntimeError("SCREENDUMP_REFUSED: " + str(response["error"]))
                size = image.stat().st_size
                total += size
                manifest.write(json.dumps({"frame": count, "start_ns": before, "end_ns": after,
                                           "bytes": size, "file": str(image.relative_to(out))}) + "\n")
                manifest.flush()
                count += 1
                if total >= a.max_bytes:
                    raise RuntimeError("CAPTURE_BUDGET_REACHED")
                # No catch-up burst after a slow copy. Actual start/end gaps are in the manifest.
                next_frame = max(next_frame + a.interval, time.monotonic())
        status = "OBSERVED"
    except (OSError, EOFError, RuntimeError, ValueError, queue.Full) as error:
        errors.append(str(error))
    finally:
        qmp_events.close()
        if stream:
            stream.close()
        if sock:
            sock.close()
        work.put(None)
        worker.join(timeout=110)  # at most five bounded diagnostic invocations
        if worker.is_alive():
            errors.append("STORE_WORKER_DID_NOT_FINISH")
        missing = [phase for phase in PHASES if phase not in queued]
        unavailable = [r["phase"] for r in finished if r["status"] != "CAPTURED"]
        report = {"status": status, "revision": a.revision, "frames": count, "bytes": total,
                  "missing_phases": missing, "unavailable_store_phases": unavailable,
                  "errors": errors, "hardware_verdict": "NOT_JUDGED",
                  "samples": "triggered reads; inspect start/end times for phase races"}
        (out / "capture-summary.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
