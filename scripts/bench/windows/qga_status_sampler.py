#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-side status sampler for one boundary run: QGA guest-exec (read-only PowerShell), one JSON
sample per attempt into <out>/run<N>-status-<k>.json. usage: sampler.py RUN OUTDIR [SECONDS]"""
import subprocess, sys, time, json, os, pathlib
run, out = sys.argv[1], pathlib.Path(sys.argv[2])
total = float(sys.argv[3]) if len(sys.argv) > 3 else 200
sock = f'/var/lib/kf-windows-20261005/boundary-kayfabe-{run}/qga.sock'
qmp = '/var/lib/kf-windows-20261005/boundary-tools/qmp.py'
PS = r'''
$ErrorActionPreference='Continue'
$os=Get-CimInstance Win32_OperatingSystem
$d=@(Get-CimInstance Win32_VideoController | Select-Object Name,PNPDeviceID,ConfigManagerErrorCode,Status)
$smi=Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue
$so=''; $rc=-1
if($smi){ $so=(& $smi.Source --query-gpu=name,driver_version --format=csv,noheader 2>&1 | Out-String); $rc=$LASTEXITCODE }
[ordered]@{time_utc=(Get-Date).ToUniversalTime().ToString('o'); uptime_seconds=((Get-Date)-$os.LastBootUpTime).TotalSeconds; display=$d; nvidia_smi=[ordered]@{available=[bool]$smi; exit_code=$rc; stdout=$so}} | ConvertTo-Json -Depth 4
'''
start = time.time(); k = 0
print('SAMPLER_START', time.strftime('%FT%T'), flush=True)
while time.time() - start < total:
    if not os.path.exists(sock):
        time.sleep(2); continue
    pr = subprocess.run(['python3', qmp, sock, 'qga-ping'], capture_output=True)
    if pr.returncode != 0:
        time.sleep(3); continue
    k += 1
    try:
        pr = subprocess.run(['timeout', '70', 'python3', qmp, sock, 'qga-exec', 'powershell.exe', '-NoProfile', '-Command', PS],
                            capture_output=True, text=True, timeout=80)
        txt = pr.stdout if pr.stdout.strip() else f'{{"error":"rc={pr.returncode}","stderr":{json.dumps(pr.stderr[-300:])}}}'
    except subprocess.TimeoutExpired:
        txt = '{"error":"sampler timeout"}'
    (out / f'run{run}-status-{k:02d}.json').write_text(txt)
    print('SAMPLE', k, f'{time.time()-start:.0f}s', len(txt), flush=True)
    time.sleep(8)
print('SAMPLER_EXIT', time.strftime('%FT%T'), flush=True)
