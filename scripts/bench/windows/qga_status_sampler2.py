#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-side status sampler v2 for one boundary run (QGA guest-exec, read-only PowerShell).
Per attempt: display adapters (ConfigManagerErrorCode), nvidia-smi, and the System-log events since boot
from nvlddmkm / Display / Kernel-Power / BugCheck / WHEA / Kernel-PnP (counts and first lines).
At the end of the window: ACPI power-down so the guest flushes its logs (QMP system_powerdown).
usage: qga_status_sampler2.py RUN OUTDIR SECONDS [WORKLOAD_AT_SECONDS [POWERSHELL_FILE]]
With DXDIAG_AT_SECONDS the sampler runs `dxdiag /t` once at that time (a light D3D9/11/12 device user on the
NVIDIA adapter through the user-mode driver) and stores the matching lines in run<N>-dxdiag.txt."""
import subprocess, sys, time, json, os, pathlib
run, out, total = sys.argv[1], pathlib.Path(sys.argv[2]), float(sys.argv[3])
dxat = float(sys.argv[4]) if len(sys.argv) > 4 else None
wlfile = sys.argv[5] if len(sys.argv) > 5 else None   # a PowerShell file run instead of dxdiag
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
DX = r'''
$f='C:\Windows\Temp\kf-dxdiag.txt'; Remove-Item $f -ErrorAction SilentlyContinue
$p=Start-Process dxdiag -ArgumentList "/t $f" -PassThru -WindowStyle Hidden
if(-not $p.WaitForExit(200000)){ 'DXDIAG_TIMEOUT'; try{$p.Kill()}catch{} }
'DXDIAG_EXIT=' + $p.ExitCode
if(Test-Path $f){ Select-String -Path $f -Pattern 'Card name|Driver Version|DDI Version|Feature Levels|Driver Model|Display Memory|DirectX Features|Problems found|Notes:|No problems|Direct3D|DirectDraw|AGP|Test' | ForEach-Object { $_.Line.Trim() } }
'''
def dxdiag():
    t = time.time()
    script = open(wlfile).read() if wlfile else DX
    try:
        pr = subprocess.run(['timeout', '260', 'python3', qmp, sock, 'qga-exec', 'powershell.exe', '-NoProfile', '-Command', script],
                            capture_output=True, text=True, timeout=270)
        txt = pr.stdout + ('\nSTDERR ' + pr.stderr[-300:] if pr.stderr else '') + f'\nRC={pr.returncode}'
    except subprocess.TimeoutExpired:
        txt = 'sampler timeout'
    (out / f'run{run}-{"workload" if wlfile else "dxdiag"}.txt').write_text(txt)
    print('DXDIAG', f'{time.time()-t:.0f}s', len(txt), flush=True)
start = time.time(); k = 0; dxdone = False
print('SAMPLER_START', time.strftime('%FT%T'), flush=True)
while time.time() - start < total:
    if not os.path.exists(sock):
        time.sleep(2); continue
    if subprocess.run(['python3', qmp, sock, 'qga-ping'], capture_output=True).returncode != 0:
        time.sleep(3); continue
    if dxat is not None and not dxdone and time.time() - start >= dxat:
        dxdone = True; dxdiag()
    k += 1; sample(k); time.sleep(10)
k += 1; sample(k)   # final sample at the end of the window
r = subprocess.run(['python3', qmp, qmps, 'cmd', 'system_powerdown'], capture_output=True, text=True)
print('POWERDOWN rc', r.returncode, r.stdout[:100], flush=True)
print('SAMPLER_EXIT', time.strftime('%FT%T'), flush=True)
