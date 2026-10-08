# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# last_crash_probe.ps1 — read-only: the guest's last bugcheck (System log, WER/BugCheck 1001), display
# TDR / driver-recovery events (Display 4101, nvlddmkm), and the boot times, one line each with a
# "CRASH" prefix. Run through QGA as SYSTEM (qga_run_ps.py RUN this.ps1). 2026-10-08, run76.
$ErrorActionPreference = 'SilentlyContinue'
function P($s) { Write-Output ("CRASH " + $s) }
$since = (Get-Date).AddHours(-2)
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$since} -MaxEvents 400 |
  Where-Object { $_.Id -in 41,1001,4101,6008,13,14,153 -or $_.ProviderName -match 'nvlddmkm|Display|BugCheck|WER' } |
  Sort-Object TimeCreated | ForEach-Object {
    $m = ($_.Message -replace "`r?`n", ' ')
    if ($m.Length -gt 300) { $m = $m.Substring(0, 300) }
    P ("{0:HH:mm:ss} id={1} src={2} {3}" -f $_.TimeCreated, $_.Id, $_.ProviderName, $m)
  }
Get-ChildItem C:\Windows\Minidump -ErrorAction SilentlyContinue | ForEach-Object { P ("minidump " + $_.Name + " " + $_.LastWriteTime) }
P ("lastboot " + (Get-CimInstance Win32_OperatingSystem).LastBootUpTime)
Get-CimInstance Win32_VideoController | ForEach-Object { P ("video name='{0}' status={1} err={2} mode={3}x{4}" -f $_.Name, $_.Status, $_.ConfigManagerErrorCode, $_.CurrentHorizontalResolution, $_.CurrentVerticalResolution) }
