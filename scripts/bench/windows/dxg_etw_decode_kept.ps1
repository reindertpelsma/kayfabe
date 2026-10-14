# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# winpass: decode the newest C:\kf\kfdxg-stall-*.etl (stopped by etwstopnow.ps1 DURING the stall, so it
# keeps its tail) with the inbox tracerpt; compact lines as dxg_etw_stop.ps1 prints them. Read-only apart from C:\kf.
$ErrorActionPreference = 'Continue'
function P($s) { Write-Output ("ETW " + $s) }
$etl = Get-ChildItem C:\kf\kfdxg-stall-*.etl -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $etl) { P "no kept etl"; exit 0 }
P ("etl " + $etl.FullName + " " + $etl.Length)
Remove-Item C:\kf\dxgk.csv -ErrorAction SilentlyContinue
tracerpt $etl.FullName -o C:\kf\dxgk.csv -of CSV -y 2>&1 | Out-Null
if (-not (Test-Path C:\kf\dxgk.csv)) { P "tracerpt produced nothing"; exit 0 }
$lines = Get-Content C:\kf\dxgk.csv
P ("csv lines " + $lines.Count)
$want = @{ '22'='P'; '177'='S'; '176'='E'; '175'='I'; '179'='QS'; '180'='QE'; '178'='QI'; '245'='QI2'; '20'='CS'; '17'='VD'; '181'='VI'; '552'='SIG'; '300'='UNW'; '238'='UQP'; '30'='CTX' }
$hist = @{}
for ($i = 1; $i -lt $lines.Count; $i++) {
  $f = $lines[$i].Split(',')
  if ($f.Count -lt 20) { continue }
  $id = $f[2].Trim()
  if ($hist.ContainsKey($id)) { $hist[$id]++ } else { $hist[$id] = 1 }
  $tag = if ($want.ContainsKey($id)) { $want[$id] } else { 'ID' + $id }
  $ud = (($f[19..($f.Count - 1)]) -join ',') -replace '\s+', ' '
  if ($ud.Length -gt 300) { $ud = $ud.Substring(0, 300) }
  Write-Output ("D " + $tag + " t=" + $f[16].Trim() + " pid=" + $f[9].Trim() + " cpu=" + $f[11].Trim() + " " + $ud)
}
$hist.GetEnumerator() | Sort-Object Value -Descending | ForEach-Object { P ("hist id " + $_.Key + " " + $_.Value) }
