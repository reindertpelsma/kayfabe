# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxg_etw_stop.ps1 — DIAGNOSTIC (2026-10-08, after run84): stop the boot-time DxgKrnl session armed by
# dxg_etw_arm.ps1, disarm it, keep this boot's .etl under its own name, decode it with the inbox
# tracerpt, and print the scheduler's events as compact lines (`D <tag> t=<FILETIME> pid= cpu= <user
# data>`, at most 20000) plus a per-ID histogram. Event IDs as the guest's own DxgKrnl manifest names
# them (wevtutil gp, run86): 22 AttemptPreemption, 175/176/177 DmaPacket info/stop/start, 178/245
# QueuePacket info, 179/180 QueuePacket start/stop, 20 UpdateContextStatus, 17 VSyncDPC, 181
# VSyncInterrupt, 552 VidSchiCompleteSignalCommmand, 300 UnwaitCpuWaiter, 238 UnwaitQueuePacket,
# 30 Context. Run through QGA as SYSTEM. Read-only apart from C:\kf and the disarm.
$ErrorActionPreference = 'Continue'
function P($s) { Write-Output ("ETW " + $s) }
$s = logman stop kfdxg -ets 2>&1
P ("stop: " + ($s -join ' '))
# ⊘ (run86) the autologger restarts at the next boot and reuses the name: disarm it, and keep this
# boot's trace under its own name.
logman delete "autosession\kfdxg" 2>&1 | Out-Null
$etl0 = Get-ChildItem C:\kf\kfdxg.etl -ErrorAction SilentlyContinue
if (-not $etl0) { P "no etl"; exit 0 }
$keep = "C:\kf\kfdxg-" + (Get-Date -Format 'HHmmss') + ".etl"
Move-Item $etl0.FullName $keep
$etl = Get-Item $keep
P ("etl " + $etl.FullName + " " + $etl.Length)
Remove-Item C:\kf\dxg.csv -ErrorAction SilentlyContinue
tracerpt $etl.FullName -o C:\kf\dxg.csv -of CSV -y 2>&1 | Out-Null
if (-not (Test-Path C:\kf\dxg.csv)) { P "tracerpt produced nothing"; exit 0 }
$lines = Get-Content C:\kf\dxg.csv
P ("csv lines " + $lines.Count)
$want = @{ '22'='P'; '177'='S'; '176'='E'; '175'='I'; '179'='QS'; '180'='QE'; '178'='QI'; '245'='QI2'; '20'='CS'; '17'='VD'; '181'='VI'; '552'='SIG'; '300'='UNW'; '238'='UQP'; '30'='CTX' }
$hist = @{}
$n = 0
for ($i = 1; $i -lt $lines.Count; $i++) {
  $f = $lines[$i].Split(',')
  if ($f.Count -lt 20) { continue }
  $id = $f[2].Trim()
  if ($hist.ContainsKey($id)) { $hist[$id]++ } else { $hist[$id] = 1 }
  if (-not $want.ContainsKey($id)) { continue }
  if ($n -ge 20000) { continue }
  $ud = (($f[19..($f.Count - 1)]) -join ',') -replace '\s+', ' '
  if ($ud.Length -gt 300) { $ud = $ud.Substring(0, 300) }
  Write-Output ("D " + $want[$id] + " t=" + $f[16].Trim() + " pid=" + $f[9].Trim() + " cpu=" + $f[11].Trim() + " " + $ud)
  $n++
}
$hist.GetEnumerator() | Sort-Object Value -Descending | ForEach-Object { P ("hist id " + $_.Key + " " + $_.Value) }
# ★ (run88) Profiler keyword: 105 = a DDI entered, 106 = left (per thread, nested). What is still
# entered at the end of the trace is a call held in the driver.
$stk = @{}
for ($i = 1; $i -lt $lines.Count; $i++) {
  $f = $lines[$i].Split(',')
  if ($f.Count -lt 20) { continue }
  $id = $f[2].Trim()
  if ($id -ne '105' -and $id -ne '106') { continue }
  $tid = $f[10].Trim()
  if (-not $stk.ContainsKey($tid)) { $stk[$tid] = New-Object System.Collections.Generic.List[string] }
  if ($id -eq '105') { $stk[$tid].Add(($f[16].Trim() + ' ' + (($f[19..($f.Count - 1)] -join ',') -replace '\s+', ' '))) }
  elseif ($stk[$tid].Count -gt 0) { $stk[$tid].RemoveAt($stk[$tid].Count - 1) }
}
foreach ($k in $stk.Keys) { foreach ($e in $stk[$k]) { P ("DDI-ENTERED-NEVER-LEFT tid=" + $k + " t=" + $e) } }
