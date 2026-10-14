# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxg_csv_summary.ps1 — DIAGNOSTIC (2026-10-08, after run86): disarm the boot-time DxgKrnl session of
# dxg_etw_arm.ps1, then summarise C:\kf\dxg.csv (tracerpt's decode of the PRE-TDR trace, written by
# dxg_etw_stop.ps1) by event ID, naming each ID from the guest's own DxgKrnl manifest (wevtutil gp):
# task, opcode and the template's field names. Then the last 40 events of each ID in $Ids with their
# user data. Bounded output; read-only apart from the disarm. Run through QGA as SYSTEM.
$ErrorActionPreference = 'SilentlyContinue'
function P($s) { Write-Output ("CSV " + $s) }
logman stop kfdxg -ets 2>&1 | Out-Null
logman delete "autosession\kfdxg" 2>&1 | Out-Null
P ("autologger: " + ((logman query "autosession\kfdxg" 2>&1) -join ' ').Substring(0, 60))
[xml]$m = wevtutil gp Microsoft-Windows-DxgKrnl /ge:true /gm:true /f:xml
$names = @{}
foreach ($e in $m.provider.events.event) {
  $names["$($e.value)"] = "task=$($e.task) op=$($e.opcode) tmpl=$($e.template) msg=$($e.message)"
}
$tmpl = @{}
foreach ($t in $m.provider.templates.template) { $tmpl["$($t.tid)"] = (($t.data | ForEach-Object { $_.name }) -join ',') }
$lines = Get-Content C:\kf\dxg.csv
P ("csv lines " + $lines.Count + " first " + $lines[1].Substring(0, [Math]::Min(200, $lines[1].Length)))
$byId = @{}
for ($i = 1; $i -lt $lines.Count; $i++) {
  $f = $lines[$i].Split(',')
  if ($f.Count -lt 4) { continue }
  $id = $f[2].Trim()
  if (-not $byId.ContainsKey($id)) { $byId[$id] = New-Object System.Collections.Generic.List[int] }
  $byId[$id].Add($i)
}
foreach ($k in ($byId.Keys | Sort-Object { $byId[$_].Count } -Descending)) {
  $n = $names[$k]; $tf = ''
  if ($n -match 'tmpl=(\S+)') { $tf = $tmpl[$Matches[1]] }
  $first = $lines[$byId[$k][0]].Split(',')[20]
  P ("id " + $k + " count " + $byId[$k].Count + " " + $n + " fields=" + $tf)
}
$Ids = @(Get-Content C:\kf\ids.txt -ErrorAction SilentlyContinue)
foreach ($k in $Ids) {
  if (-not $byId.ContainsKey($k)) { continue }
  $sel = $byId[$k] | Select-Object -Last 40
  foreach ($i in $sel) { $t = $lines[$i]; if ($t.Length -gt 600) { $t = $t.Substring(0, 600) }; P ("ev " + $k + " " + ($t -replace '\s+', ' ')) }
}
