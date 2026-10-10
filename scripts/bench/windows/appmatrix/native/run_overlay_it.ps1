# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# run_overlay_it.ps1 -Tag NAME -ProbeArgs "--scenario steady ..." [-TimeoutSec 120] -- run C:\kf\kf_overlayprobe.exe in the INTERACTIVE
# session (scheduled task /it, as the app-matrix adapter does) and wait for its stdout (C:\kf\ovl\NAME.txt, last line `KFOVL RESULT`).
# Prints the file's KFOVL lines except the (large) JSON line, then `KFOVLRUN rc=<n>`. The JSON is also in C:\kf\ovl\NAME.json.
param([Parameter(Mandatory = $true)][string]$Tag, [string]$ProbeArgs = '', [int]$TimeoutSec = 120)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force C:\kf\ovl | Out-Null
$txt = "C:\kf\ovl\$Tag.txt"; $json = "C:\kf\ovl\$Tag.json"; $cmd = "C:\kf\ovl\run_$Tag.cmd"
Remove-Item $txt, $json -ErrorAction SilentlyContinue
Set-Content -Path $cmd -Encoding ASCII -Value ("@echo off`r`nC:\kf\kf_overlayprobe.exe $ProbeArgs --out $json > $txt 2>&1`r`necho EXITCODE %ERRORLEVEL% >> $txt`r`n")
$user = (Get-Process explorer -ErrorAction SilentlyContinue | Select-Object -First 1).UserName
if (-not $user) { $user = "$env:COMPUTERNAME\vast" }
schtasks /delete /tn "kfovl_$Tag" /f 2>&1 | Out-Null
schtasks /create /tn "kfovl_$Tag" /tr $cmd /sc once /st 00:00 /it /ru $user /rl highest /f 2>&1 | Out-Null
schtasks /run /tn "kfovl_$Tag" 2>&1 | Out-Null
$end = (Get-Date).AddSeconds($TimeoutSec)
while ((Get-Date) -lt $end) { if ((Test-Path $txt) -and (Select-String -Path $txt -Pattern '^EXITCODE' -Quiet)) { break }; Start-Sleep -Milliseconds 500 }
schtasks /delete /tn "kfovl_$Tag" /f 2>&1 | Out-Null
if (Test-Path $txt) { Get-Content $txt | Where-Object { $_ -notmatch '^KFOVL JSON ' } | ForEach-Object { [string]$_ }; } else { 'NO OUTPUT FILE' }
