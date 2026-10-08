# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# vdr_etw_extract.ps1 -- (2026-10-08, VFIO DVI reference) decode the newest kept DxgKrnl .etl (dxg_etw_stop_now.ps1)
# with the inbox tracerpt to C:\kf\vdr-dxg.csv, then write the scheduler events as compact lines (the tags of
# dxg_etw_stop.ps1, plus every other id as ID<n>) to C:\kf\vdr-etw.txt and zip it to C:\kf\vdr-etw.zip, so the host
# fetches one file (a full decode is too large for QGA's captured output). Prints only a summary. Started detached
# (it can take minutes); writes C:\kf\vdr-etw.done when finished. Read-only apart from C:\kf.
$ErrorActionPreference = 'Continue'
$etl = Get-ChildItem C:\kf\kfdxg-stall-*.etl -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
Remove-Item C:\kf\vdr-dxg.csv, C:\kf\vdr-etw.txt, C:\kf\vdr-etw.zip, C:\kf\vdr-etw.done -ErrorAction SilentlyContinue
$log = New-Object System.Collections.Generic.List[string]
$log.Add('etl ' + $etl.FullName + ' ' + $etl.Length + ' start ' + (Get-Date).ToUniversalTime().ToString('o'))
tracerpt $etl.FullName -o C:\kf\vdr-dxg.csv -of CSV -y 2>&1 | Out-Null
$log.Add('tracerpt done ' + (Get-Date).ToUniversalTime().ToString('o'))
$want = @{ '22'='P'; '177'='S'; '176'='E'; '175'='I'; '179'='QS'; '180'='QE'; '178'='QI'; '245'='QI2'; '20'='CS'; '17'='VD'; '181'='VI'; '552'='SIG'; '300'='UNW'; '238'='UQP'; '30'='CTX' }
$hist = @{}
$w = New-Object System.IO.StreamWriter('C:\kf\vdr-etw.txt')
$r = New-Object System.IO.StreamReader('C:\kf\vdr-dxg.csv')
$hdr = $r.ReadLine(); $w.WriteLine('HDR ' + $hdr)
while (($line = $r.ReadLine()) -ne $null) {
  $f = $line.Split(',')
  if ($f.Count -lt 20) { continue }
  $id = $f[2].Trim()
  if ($hist.ContainsKey($id)) { $hist[$id]++ } else { $hist[$id] = 1 }
  $tag = if ($want.ContainsKey($id)) { $want[$id] } else { 'ID' + $id }
  $ud = (($f[19..($f.Count - 1)]) -join ',') -replace '\s+', ' '
  if ($ud.Length -gt 400) { $ud = $ud.Substring(0, 400) }
  $w.WriteLine('D ' + $tag + ' t=' + $f[16].Trim() + ' pid=' + $f[9].Trim() + ' tid=' + $f[10].Trim() + ' cpu=' + $f[11].Trim() + ' ' + $ud)
}
$r.Close(); $w.Close()
Compress-Archive -Path C:\kf\vdr-etw.txt -DestinationPath C:\kf\vdr-etw.zip -Force
$hist.GetEnumerator() | Sort-Object Value -Descending | ForEach-Object { $log.Add('hist id ' + $_.Key + ' ' + $_.Value) }
$log.Add('zip ' + (Get-Item C:\kf\vdr-etw.zip).Length + ' end ' + (Get-Date).ToUniversalTime().ToString('o'))
$log | Set-Content C:\kf\vdr-etw.done
