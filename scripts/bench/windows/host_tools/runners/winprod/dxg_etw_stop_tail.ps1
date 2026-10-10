# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxg_etw_stop_tail.ps1 — DIAGNOSTIC (2026-10-10, TDR hunt): like dxg_etw_stop.ps1, for a CIRCULAR live DxgKrnl session
# (`logman create trace kfdxg -p Microsoft-Windows-DxgKrnl 0x1 5 -o C:\kf\kfdxg.etl -f bincirc -max 512 -ets`) stopped right
# after the first TDR: the decoded scheduler events are printed as the TAIL (last 25000 wanted events, `D <tag> t=.. pid=.. cpu=..
# <user data>`), because the first-20000 rule of dxg_etw_stop.ps1 cut off the end of a 512 MiB trace before the stall. Read-only apart from C:\kf.
$ErrorActionPreference = 'Continue'
function P($s) { Write-Output ("ETW " + $s) }
$s = logman stop kfdxg -ets 2>&1
P ("stop: " + ($s -join ' ') + " at " + (Get-Date).ToUniversalTime().ToString('o'))
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
$out = New-Object System.Collections.Generic.List[string]
for ($i = 1; $i -lt $lines.Count; $i++) {
  $f = $lines[$i].Split(',')
  if ($f.Count -lt 20) { continue }
  $id = $f[2].Trim()
  if ($hist.ContainsKey($id)) { $hist[$id]++ } else { $hist[$id] = 1 }
  if (-not $want.ContainsKey($id)) { continue }
  $ud = (($f[19..($f.Count - 1)]) -join ',') -replace '\s+', ' '
  if ($ud.Length -gt 300) { $ud = $ud.Substring(0, 300) }
  $out.Add("D " + $want[$id] + " t=" + $f[16].Trim() + " pid=" + $f[9].Trim() + " cpu=" + $f[11].Trim() + " " + $ud)
}
$skip = [Math]::Max(0, $out.Count - 25000)
P ("wanted events " + $out.Count + ", printing the last " + ($out.Count - $skip))
for ($i = $skip; $i -lt $out.Count; $i++) { Write-Output $out[$i] }
$hist.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 25 | ForEach-Object { P ("hist id " + $_.Key + " " + $_.Value) }
