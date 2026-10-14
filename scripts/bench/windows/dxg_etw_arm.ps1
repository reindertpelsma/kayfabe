# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxg_etw_arm.ps1 — DIAGNOSTIC (2026-10-08, after run84; the guest's own view of the TDR): arm, for the
# NEXT boot of this disk, (1) a boot-time ETW session of the Microsoft-Windows-DxgKrnl provider (all
# keywords, level 5, file C:\kf\kfdxg.etl, bounded to 512 MiB) and (2) TdrDelay = TdrDdiDelay = 30 s
# (run79's setting) so the trace can be stopped between the preempt and the teardown. Read back both.
# Run through QGA as SYSTEM (qga_run_ps.py RUN this.ps1). Nothing outside the guest is touched.
$ErrorActionPreference = 'Continue'
function P($s) { Write-Output ("ETWARM " + $s) }
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
# ★ (run88) TdrDdiDelay apart from TdrDelay: C:\kf\tdrddidelay.txt (if present) sets it, so a DDI timeout
# (a thread held in the driver) and a GPU-scheduler timeout land at different delays.
$ddi = 30
if (Test-Path C:\kf\tdrddidelay.txt) { $ddi = [int](Get-Content C:\kf\tdrddidelay.txt | Select-Object -First 1) }
Set-ItemProperty -Path $gd -Name TdrDelay -Type DWord -Value 30
Set-ItemProperty -Path $gd -Name TdrDdiDelay -Type DWord -Value $ddi
P ("TdrDelay=" + (Get-ItemProperty $gd).TdrDelay + " TdrDdiDelay=" + (Get-ItemProperty $gd).TdrDdiDelay)
logman delete "autosession\kfdxg" 2>&1 | Out-Null
# ⊘ (run86) all keywords: the Profiler keyword alone wrote ~30k events in the first 2 s and the decoded
# trace ended there. Keyword 0x1 (Base: DmaPacket, QueuePacket, AttemptPreemption, context status,
# VSync) only, flushed every second. The .etl of a boot is renamed by dxg_etw_stop.ps1 before the
# next boot can reuse the name.
# ★ (run88) C:\kf\keywords.txt (if present) overrides, e.g. 0x3 = Base + Profiler (DDI enter/exit,
# events 105/106: an enter with no exit names a call held in the driver).
$kw = '0x1'
if (Test-Path C:\kf\keywords.txt) { $kw = (Get-Content C:\kf\keywords.txt | Select-Object -First 1).Trim() }
$o = logman create trace "autosession\kfdxg" -p "Microsoft-Windows-DxgKrnl" $kw 5 -o C:\kf\kfdxg.etl -bs 1024 -nb 64 512 -ft 1 -max 512 2>&1
P ("create: " + ($o -join ' '))
$q = logman query "autosession\kfdxg" 2>&1
$q | ForEach-Object { P $_ }
