#!/usr/bin/env python3
"""Capture bounded host evidence on guest PresentMon notifications.

Notifications select no PID or path: target identity comes from the lane's
active run receipt. The existing guest monitor has a global cooldown, so these
live traces supplement the complete per-PID PresentMon analysis at run end.
"""
import argparse
import json
import os
from pathlib import Path
import re
import resource
import signal
import socket
import subprocess
import sys
import time


def parse_trigger(text):
    if len(text) > 4096 or "\0" in text:
        return None
    m = re.search(r"\bTRIGGER ([A-Za-z0-9_.-]+) silent ([0-9]+(?:\.[0-9]+)?)s", text)
    if not m or float(m[2]) < 4:
        return None
    return m[1], float(m[2])


def perf_command(tids, out):
    if not tids:
        return None
    if len(tids) > 128 or any(type(tid) is not int or tid <= 0 for tid in tids):
        raise ValueError("invalid thread selection")
    filt = " || ".join(f"(prev_pid=={tid} || next_pid=={tid})" for tid in tids)
    return ["perf", "record", "-a", "-e", "sched:sched_switch", "--filter", filt,
            "-g", "-o", str(out / "offcpu.data")]


def snapshot(pid):
    rows = []
    for p in Path(f"/proc/{pid}/task").glob("*"):
        try:
            comm = (p / "comm").read_text().strip()
            if not comm.startswith("kf3-"):
                continue
            row = dict(tid=int(p.name), comm=comm)
            for name in ("stat", "wchan", "syscall", "stack"):
                try:
                    row[name] = (p / name).read_text()
                except OSError as e:
                    row[name] = type(e).__name__
            rows.append(row)
        except OSError:
            pass
    return dict(epoch=time.time(), pid=pid, threads=rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True)
    ap.add_argument("--scripts", required=True)
    ap.add_argument("--kvmthreads", required=True)
    ap.add_argument("--port", type=int, default=39021)
    ap.add_argument("--seconds", type=int, default=7200)
    ap.add_argument("--perf", action="store_true", help="also record filtered scheduler callchains")
    ap.add_argument("--userd-module", type=Path, help="existing read-only USERD probe module")
    ap.add_argument("--lowmem", type=lambda x: int(x, 0), help="verified guest RAM split for Translated probe")
    args = ap.parse_args()
    if args.userd_module and args.lowmem is None:
        ap.error("--userd-module requires the verified --lowmem split")
    root = Path(args.root)
    root.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, args.scripts)
    import identity
    stop = False
    def stopped(*_):
        nonlocal stop
        stop = True
    signal.signal(signal.SIGTERM, stopped)
    signal.signal(signal.SIGINT, stopped)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", args.port))
    sock.settimeout(1)
    current, cpu, active, sequence, notifications = None, None, [], 0, 0
    last_snapshot = 0
    full_journals = set()
    progress = None
    progress_tail = ""
    translated = None
    until = time.monotonic() + args.seconds
    try:
        with (root / "live-events.jsonl").open("a", buffering=1) as events:
            while not stop and time.monotonic() < until:
                candidates = []
                for f in root.glob("*/run.json"):
                    try:
                        n = json.loads(f.read_text())["run"]
                        pid = int((f.parent / "qpid").read_text())
                        argv = Path(f"/proc/{pid}/cmdline").read_bytes().decode().strip("\0").split("\0")
                        if identity.is_qemu(argv, n):
                            candidates.append((f.stat().st_mtime, pid, f.parent))
                    except (OSError, ValueError, KeyError):
                        pass
                selected = max(candidates, default=None)
                target = (selected[1], selected[2]) if selected else None
                if target != current:
                    if translated and translated.poll() is None:
                        translated.terminate()
                        translated.wait(timeout=5)
                    translated = None
                    if progress:
                        progress.close()
                        progress = None
                    progress_tail = ""
                    if cpu and cpu.poll() is None:
                        cpu.terminate()
                        cpu.wait(timeout=5)
                    current = target
                    if current:
                        pid, out = current
                        cpu = subprocess.Popen([sys.executable, args.kvmthreads, str(pid), str(args.seconds), str(out / "thread-cpu.tsv")])
                        try:
                            stderr = Path(f"/proc/{pid}/fd/2").resolve()
                            if stderr.is_file():
                                progress = stderr.open(errors="replace")
                        except OSError:
                            pass  # the verified VM may have just exited
                        if progress and args.userd_module:
                            translated = subprocess.Popen([
                                sys.executable, str(Path(__file__).with_name("translated_probe.py")),
                                "--pid", str(pid), "--log", str(stderr),
                                "--out", str(out / "translated.jsonl"),
                                "--userd-module", str(args.userd_module), "--lowmem", str(args.lowmem),
                                "--seconds", str(args.seconds),
                            ], stdout=(out / "translated-status.txt").open("w"), stderr=subprocess.STDOUT)
                if progress and current:
                    lines = (progress_tail + progress.read(4 * 1024 * 1024)).split("\n")
                    progress_tail = lines.pop()[-16384:]
                    selected_lines = [line[:16384] for line in lines if "PROGRESS_" in line or "worker_stuck" in line]
                    journal = current[1] / "progress-live.jsonl"
                    if selected_lines and (not journal.exists() or journal.stat().st_size < 8 * 1024 * 1024):
                        with journal.open("a") as f:
                            for line in selected_lines:
                                f.write(json.dumps(dict(observed_epoch=time.time(), line=line)) + "\n")
                # Retain quiet/blocked threads too, including the seconds before
                # the live monitor can first detect a four-second silence.
                if current and time.monotonic() - last_snapshot >= 1:
                    last_snapshot = time.monotonic()
                    pid, out = current
                    journal = out / "thread-state.jsonl"
                    if not journal.exists() or journal.stat().st_size < 64 * 1024 * 1024:
                        with journal.open("a") as f:
                            f.write(json.dumps(snapshot(pid)) + "\n")
                    elif journal not in full_journals:
                        full_journals.add(journal)
                        events.write(json.dumps(dict(snapshot_limit=True, epoch=time.time(), run=out.name)) + "\n")
                for proc, end, out, pid in list(active):
                    if time.monotonic() >= end or proc.poll() is not None:
                        if proc.poll() is None:
                            proc.terminate()
                        proc.wait(timeout=5)
                        (out / "after.json").write_text(json.dumps(snapshot(pid), indent=2))
                        active.remove((proc, end, out, pid))
                try:
                    raw, _ = sock.recvfrom(4097)
                except socket.timeout:
                    continue
                text = raw.decode("utf-8", errors="replace")
                notifications += 1
                if notifications > 4096:
                    events.write(json.dumps(dict(notification_limit=True, epoch=time.time())) + "\n")
                    break
                parsed = parse_trigger(text)
                entry = dict(epoch=time.time(), notification=text, parsed=parsed, run=str(current[1].name) if current else None)
                events.write(json.dumps(entry) + "\n")
                if not parsed or not current or active or sequence >= 128:
                    continue
                pid, run_dir = current
                sequence += 1
                out = run_dir / f"host-freeze-{sequence:02d}"
                out.mkdir()
                state = snapshot(pid)
                (out / "before.json").write_text(json.dumps(state, indent=2))
                tids = [r["tid"] for r in state["threads"] if any(w in r["comm"] for w in ("worker", "drain", "regs", "act", "va"))]
                if tids:
                    command = ["strace", "-f", "-ttt", "-T", "-e", "trace=ioctl,futex", "-o", str(out / "strace.txt")]
                    for tid in tids:
                        command += ["-p", str(tid)]
                    def limit_trace():
                        resource.setrlimit(resource.RLIMIT_FSIZE, (32 * 1024 * 1024, 32 * 1024 * 1024))
                    proc = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=(out / "strace-status.txt").open("w"), preexec_fn=limit_trace)
                    active.append((proc, time.monotonic() + 10, out, pid))
                    if args.perf:
                        perf = subprocess.Popen(perf_command(tids, out), stdout=subprocess.DEVNULL,
                                                stderr=(out / "perf-status.txt").open("w"), preexec_fn=limit_trace)
                        active.append((perf, time.monotonic() + 10, out, pid))
    finally:
        if translated and translated.poll() is None:
            translated.terminate()
            translated.wait(timeout=5)
        if progress:
            progress.close()
        for proc, _, _, _ in active:
            if proc.poll() is None:
                proc.terminate()
            proc.wait(timeout=5)
        if cpu and cpu.poll() is None:
            cpu.terminate()
            cpu.wait(timeout=5)
        sock.close()


if __name__ == "__main__":
    main()
