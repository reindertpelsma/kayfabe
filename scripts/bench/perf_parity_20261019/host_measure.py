#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""P1 host-side guest transport and measurements. Caller owns the GPU lease.

All command text is passed as an argument read from a script file. Guest artifacts
and tasks use the P1 namespace. This runner never starts/stops an arbitrary process.
"""
import argparse
import base64
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import zipfile


HERE = Path(__file__).resolve().parent
GUEST = r"C:\kf\P1"


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def agent(root, text, timeout=90):
    # Keep the exact dispatched script alongside its output and metadata.
    path = root / ("dispatch_%d.ps1" % time.time_ns())
    path.write_text(text, encoding="utf-8")
    result = subprocess.run(
        [sys.executable, str(root.parent / "kfagent_server.py"), "run", path.read_text(), str(timeout)],
        text=True, capture_output=True, timeout=timeout + 15,
    )
    path.with_suffix(".out").write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f"guest dispatch failed rc={result.returncode}: {path}")
    return result.stdout


def stage(root):
    for source in sorted(HERE.glob("*.ps1")):
        encoded = base64.b64encode(source.read_bytes()).decode("ascii")
        body = (f"New-Item -ItemType Directory -Force {GUEST} | Out-Null\n"
                f"[IO.File]::WriteAllBytes({quote(GUEST + chr(92) + source.name)}, "
                f"[Convert]::FromBase64String('{encoded}'))\n'P1_STAGED {source.name}'")
        response = agent(root, body)
        if f"P1_STAGED {source.name}" not in response:
            raise RuntimeError("guest did not confirm staged script " + source.name)
    print("P1_GUEST_STAGE_COMPLETE", flush=True)


def probe(root, tag, queue, iterations, pre_ms):
    (root / tag).mkdir(exist_ok=False)
    log = GUEST + "\\" + tag + ".txt"
    raw = GUEST + "\\" + tag + ".csv"
    args = (f"-Out {log} -Mode T -Queue {queue} -Work copy -Iter {iterations} "
            f"-FenceBase 5A5A5A5A00000000 -PreMs {pre_ms} -Raw {raw}")
    body = rf"""
& {GUEST}\sl_it.ps1 -Script {GUEST}\sl_probe.ps1 -ScriptArgs {quote(args)} -Wait 45 -Log {quote(log)} | Out-Null
$end = (Get-Date).AddSeconds(120)
while ((Get-Date) -lt $end -and -not (Select-String -Path {quote(log)} -Pattern '^GP end$' -Quiet -ErrorAction SilentlyContinue)) {{ Start-Sleep -Milliseconds 300 }}
Get-Content {quote(log)}
if (-not (Select-String -Path {quote(log)} -Pattern '^GP end$' -Quiet -ErrorAction SilentlyContinue)) {{ throw 'P1 probe did not finish' }}
& {GUEST}\sn_upload.ps1 -File {quote(raw)} -Name {quote('p1_' + tag + '_raw')}
"""
    response = agent(root, body, 180)
    (root / tag / "guest.txt").write_text(response)
    if "GP end" not in response or "TIMEOUT" in response or "GP exception" in response:
        raise RuntimeError("P1 probe failed; preserve log and stop")
    upload = root.parent / "kfagent" / "r" / ("p1_" + tag + "_raw.txt")
    (root / tag / "raw.csv").write_bytes(upload.read_bytes())
    print(response, flush=True)


def ui(root, action, fx, fy):
    args = f"-Action {action} -FX {fx} -FY {fy}"
    body = f"& {GUEST}\\sn_it.ps1 -ScriptArgs {quote(args)} -Wait 90\n"
    response = agent(root, body, 120)
    print(response, flush=True)
    if "NO LOG" in response or "NOWINDOW" in response:
        raise RuntimeError("UI task failed; inspect the screenshot before continuing")


def capture(root, tag, seconds):
    destination = root / tag
    destination.mkdir(exist_ok=False)
    start = agent(root, f"& {GUEST}\\sn_cap_start.ps1 -Tag {quote(tag)} -Keywords 0x3 -Seconds {seconds}")
    (destination / "start.txt").write_text(start)
    if "CAPSTART" not in start:
        raise RuntimeError("capture did not start")
    ui(root, "run", 0.1786, 0.845)
    # Measurement duration only: never changes an interrupt/submit deadline.
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        time.sleep(min(5, until - time.monotonic()))
    stop = agent(root, f"& {GUEST}\\sn_cap_stop.ps1 -Tag {quote(tag)}", 900)
    (destination / "stop.txt").write_text(stop)
    upload = root.parent / "kfagent" / "r" / ("p1_sn_" + tag + "_zip.txt")
    archive = destination / (tag + ".zip")
    archive.write_bytes(upload.read_bytes())
    with zipfile.ZipFile(archive) as zipped:
        for info in zipped.infolist():
            if Path(info.filename).name != info.filename:
                raise RuntimeError("unexpected archive entry")
        zipped.extractall(destination)
    for script in ("sn_extract.py", "sn_analyze.py", "sn_ddi.py"):
        subprocess.run([sys.executable, str(HERE / script), str(destination), tag], check=True)
    print("P1_CAPTURE_COMPLETE", tag, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path, help="lane directory under /data")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("stage")
    p = sub.add_parser("guest")
    p.add_argument("script", type=Path)
    p.add_argument("--timeout", type=int, default=90)
    p = sub.add_parser("probe")
    p.add_argument("tag")
    p.add_argument("--queue", type=int, choices=[0, 2, 3], default=0)
    p.add_argument("--iterations", type=int, default=3000)
    p.add_argument("--pre-ms", type=int, default=0)
    p = sub.add_parser("ui")
    p.add_argument("action", choices=["launch", "dump", "focus", "shot", "click", "run"])
    p.add_argument("--fx", type=float, default=0.1786)
    p.add_argument("--fy", type=float, default=0.845)
    p = sub.add_parser("capture")
    p.add_argument("tag")
    p.add_argument("--seconds", type=int, default=105)
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    if root.name != "codex-P1" or root.parent != Path("/data"):
        parser.error("this lane may write only its own host directory")
    if not (root / "GPU_LEASE_HELD").is_file():
        parser.error("P1 GPU lease marker is absent")
    if getattr(args, "tag", None) and not re.fullmatch(r"[a-zA-Z0-9_-]+", args.tag):
        parser.error("tag must be a single safe component")
    metadata = {"time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "arguments": sys.argv[1:]}
    (root / ("measurement_%d.json" % time.time_ns())).write_text(json.dumps(metadata, indent=2))
    if args.command == "stage":
        stage(root)
    elif args.command == "guest":
        print(agent(root, args.script.read_text(), args.timeout), flush=True)
    elif args.command == "probe":
        probe(root, args.tag, args.queue, args.iterations, args.pre_ms)
    elif args.command == "ui":
        ui(root, args.action, args.fx, args.fy)
    elif args.command == "capture":
        capture(root, args.tag, args.seconds)


if __name__ == "__main__":
    main()
