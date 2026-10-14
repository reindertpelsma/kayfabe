# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxg_etw_decode.ps1 — DIAGNOSTIC (2026-10-08, after run84): decode C:\kf\kfdxg.etl (dxg_etw_arm.ps1 /
# dxg_etw_stop.ps1) with the guest's own DxgKrnl manifest (Get-WinEvent -Path), and print a bounded
# summary: per (Id, task, opcode) counts, every event whose task names preemption, TDR, timeout, reset,
# fault, flip, vsync or present (first 400, with their properties), and the last 150 events of the
# trace with properties. Read-only. Run through QGA as SYSTEM.
$ErrorActionPreference = 'SilentlyContinue'
function P($s) { Write-Output ("DXG " + $s) }
$etl = Get-ChildItem C:\kf\kfdxg*.etl | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $etl) { P "no etl"; exit 0 }
$ev = Get-WinEvent -Path $etl.FullName -Oldest -ErrorAction SilentlyContinue
P ("events " + $ev.Count + " from " + $etl.FullName)
function Line($e) {
  $props = ($e.Properties | ForEach-Object { $v = $_.Value; if ($v -is [byte[]]) { 'bytes' + $v.Length } else { "$v" } }) -join ';'
  if ($props.Length -gt 260) { $props = $props.Substring(0, 260) }
  "{0:HH:mm:ss.fff} id={1} task={2} op={3} pid={4} tid={5} [{6}]" -f $e.TimeCreated, $e.Id, $e.TaskDisplayName, $e.OpcodeDisplayName, $e.ProcessId, $e.ThreadId, $props
}
$ev | Group-Object { "{0}/{1}/{2}" -f $_.Id, $_.TaskDisplayName, $_.OpcodeDisplayName } | Sort-Object Count -Descending |
  Select-Object -First 120 | ForEach-Object { P ("hist " + $_.Count + " " + $_.Name) }
$i = 0
foreach ($e in $ev) {
  if ("$($e.TaskDisplayName)" -match 'Preempt|Tdr|TDR|Timeout|Reset|Fault|Flip|VSync|Vsync|Present|Hung|Recover|Engine') {
    P ("hit " + (Line $e)); $i++; if ($i -ge 400) { break }
  }
}
$ev | Select-Object -Last 150 | ForEach-Object { P ("tail " + (Line $_)) }
