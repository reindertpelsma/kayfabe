#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-side status sampler v2 for one boundary run (QGA guest-exec, read-only PowerShell).
Per attempt: display adapters (ConfigManagerErrorCode), nvidia-smi, and the System-log events since boot
from nvlddmkm / Display / Kernel-Power / BugCheck / WHEA / Kernel-PnP (counts and first lines).
At the end of the window: ACPI power-down so the guest flushes its logs (QMP system_powerdown).
usage: sampler2.py RUN OUTDIR SECONDS"""
import subprocess, sys, time, json, os, pathlib
run, out, total = sys.argv[1], pathlib.Path(sys.argv[2]), float(sys.argv[3])
rd = f'/var/lib/kf-windows-20261005/boundary-kayfabe-{run}'
sock, qmps = rd + '/qga.sock', rd + '/qmp.sock'
qmp = '/var/lib/kf-windows-20261005/boundary-tools/qmp.py'
PS = r'''
$ErrorActionPreference='Continue'
$os=Get-CimInstance Win32_OperatingSystem
$d=@(Get-CimInstance Win32_VideoController | Select-Object Name,PNPDeviceID,ConfigManagerErrorCode,Status)
$smi=Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue
$so=''; $rc=-1
if($smi){ $so=(& $smi.Source --query-gpu=name,driver_version,pstate,utilization.gpu,memory.used --format=csv,noheader 2>&1 | Out-String); $rc=$LASTEXITCODE }
$ev=@(Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$os.LastBootUpTime} -ErrorAction SilentlyContinue |
  Where-Object { $_.ProviderName -match 'nvlddmkm|Display|Kernel-Power|BugCheck|WHEA|Kernel-PnP' -or $_.Id -in 4101,1001,41 } |
  Select-Object -First 12 TimeCreated,ProviderName,Id,LevelDisplayName,@{n='Msg';e={ $_.Message.Substring(0,[Math]::Min(160,$_.Message.Length)) }})
[ordered]@{time_utc=(Get-Date).ToUniversalTime().ToString('o'); uptime_seconds=((Get-Date)-$os.LastBootUpTime).TotalSeconds; display=$d; nvidia_smi=[ordered]@{available=[bool]$smi; exit_code=$rc; stdout=$so}; events=$ev} | ConvertTo-Json -Depth 4
'''
def sample(k):
    try:
        pr = subprocess.run(['timeout', '90', 'python3', qmp, sock, 'qga-exec', 'powershell.exe', '-NoProfile', '-Command', PS],
                            capture_output=True, text=True, timeout=100)
        txt = pr.stdout if pr.stdout.strip() else json.dumps({"error": f"rc={pr.returncode}", "stderr": pr.stderr[-300:]})
    except subprocess.TimeoutExpired:
        txt = '{"error":"sampler timeout"}'
    (out / f'run{run}-status-{k:02d}.json').write_text(txt)
    print('SAMPLE', k, f'{time.time()-start:.0f}s', len(txt), flush=True)
start = time.time(); k = 0
print('SAMPLER_START', time.strftime('%FT%T'), flush=True)
while time.time() - start < total:
    if not os.path.exists(sock):
        time.sleep(2); continue
    if subprocess.run(['python3', qmp, sock, 'qga-ping'], capture_output=True).returncode != 0:
        time.sleep(3); continue
    k += 1; sample(k); time.sleep(10)
k += 1; sample(k)   # final sample at the end of the window
r = subprocess.run(['python3', qmp, qmps, 'cmd', 'system_powerdown'], capture_output=True, text=True)
print('POWERDOWN rc', r.returncode, r.stdout[:100], flush=True)
print('SAMPLER_EXIT', time.strftime('%FT%T'), flush=True)
