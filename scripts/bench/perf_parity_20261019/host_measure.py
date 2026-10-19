#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""P1 host-side guest transport and measurements. Caller owns the GPU lease.

All command text is passed as an argument read from a script file. Guest artifacts
and tasks use the P1 namespace. This runner never starts/stops an arbitrary process.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time
import zipfile


HERE = Path(__file__).resolve().parent
GUEST = r"C:\kf\P1"


def require_lease(root):
    pid, parent, _, started, parent_started = (root / 'GPU_LEASE_HELD').read_text().split()
    for process, expected in [(pid, started), (parent, parent_started)]:
        os.kill(int(process), 0)
        if Path('/proc/' + process + '/stat').read_text().split()[21] != expected:
            raise RuntimeError('GPU lease process was reused')
    if Path('/proc/' + parent + '/comm').read_text().strip() != 'flock':
        raise RuntimeError('GPU lease parent is not flock')
    if b'/tmp/kayfabe-fastguest.lock' not in Path('/proc/' + parent + '/cmdline').read_bytes():
        raise RuntimeError('GPU lease parent holds an unexpected lock')


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def fence_high(tag):
    # A per-run marker prevents stale fence words from earlier probe processes
    # filling the read-only observer's candidate set in the same guest boot.
    return 0x50000000 | (int.from_bytes(hashlib.sha256(tag.encode()).digest()[:4], 'big') & 0x0fffffff)


def agent(root, text, timeout=90):
    # Keep the exact dispatched script alongside its output and metadata.
    path = root / ("dispatch_%d.ps1" % time.time_ns())
    marker = '# P1_DISPATCH_' + str(time.time_ns())
    text = marker + '\n' + text
    path.write_text(text, encoding="utf-8")
    result = subprocess.run(
        [sys.executable, str(root.parent / "kfagent_server.py"), "run", path.read_text(), str(timeout)],
        text=True, capture_output=True, timeout=timeout + 15,
    )
    path.with_suffix(".out").write_text(result.stdout + result.stderr)
    if result.returncode:
        # An unsent command must not run later in a restored owner VM. Remove
        # only this dispatch's exact command text; a sent command is retained.
        for queued in (root.parent / 'kfagent' / 'q').glob('*.json'):
            if json.loads(queued.read_text()).get('cmd') == text:
                queued.unlink()
        raise RuntimeError(f"guest dispatch failed rc={result.returncode}: {path}")
    return result.stdout


def stats_checkpoint(root):
    active = json.loads((root / 'active-arm.json').read_text())
    flags = dict(item.split('=', 1) for item in shlex.split(active['flags']) if '=' in item)
    if flags.get('KF3_DIAG_NSI_TRACE') != '1':
        return None
    dump = Path(flags['KF3_DIAG_NSI_TRACE_DUMP']).resolve()
    if not dump.is_relative_to(root):
        raise RuntimeError('diagnostic request path leaves P1 directory')
    request = Path(str(dump) + '.stats')
    output = Path(str(dump) + '.stats.out')
    with request.open('ab') as stream:
        stream.write(b'x')
    checkpoint_id = request.stat().st_size
    until = time.monotonic() + 10
    while time.monotonic() < until:
        for line in output.read_text().splitlines() if output.is_file() else []:
            if f'DIAG_NSI_STATS id={checkpoint_id} ' in line:
                match = re.search(r'backstop_found\(work=(\d+) carried=(\d+) contended=(\d+)\)', line)
                if not match:
                    raise RuntimeError('unconditional relay counters are missing from checkpoint')
                return {'id': checkpoint_id, 'monotonic_ns': time.monotonic_ns(), 'line': line,
                        'backstop_work': int(match[1]), 'backstop_carried': int(match[2]),
                        'backstop_contended': int(match[3])}
        time.sleep(.1)
    raise RuntimeError('live diagnostic checkpoint was not acknowledged; stop and inspect the request/output')


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
            f"-FenceBase {fence_high(tag):08X}00000000 -PreMs {pre_ms} -Raw {raw}")
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
    body = f"& {GUEST}\\sl_it.ps1 -ScriptArgs {quote(args)} -Wait 90\n"
    response = agent(root, body, 120)
    print(response, flush=True)
    if "NO LOG" in response or "NOWINDOW" in response:
        raise RuntimeError("UI task failed; inspect the screenshot before continuing")


def capture(root, tag, seconds, fx=0.1786, fy=0.845):
    destination = root / tag
    destination.mkdir(exist_ok=False)
    before = stats_checkpoint(root)
    (destination / 'counters-before.json').write_text(json.dumps(before, indent=2))
    start = agent(root, f"& {GUEST}\\sn_cap_start.ps1 -Tag {quote(tag)} -Keywords 0x3 -Seconds {seconds}")
    (destination / "start.txt").write_text(start)
    if "CAPSTART" not in start:
        raise RuntimeError("capture did not start")
    ui(root, "run", fx, fy)
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
    after = stats_checkpoint(root)
    (destination / 'counters-after.json').write_text(json.dumps(after, indent=2))
    if before and after and after['backstop_work'] > before['backstop_work']:
        raise RuntimeError('BACKSTOP_FOUND_WORK increased during the capture; investigate the missed event before more performance arms')
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
    require_lease(root)
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
