# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_cap_stop.ps1 -Tag NAME: stop the p1_sn ETW session and PresentMon, decode the etl with the inbox tracerpt (CSV), zip etl-csv + PresentMon csv,
# and upload the zip to the host kfagent broker (the documented kfagent result directory). Prints sizes and the HAGS registry value.
param([Parameter(Mandatory = $true)][string]$Tag)
$ErrorActionPreference = 'Continue'
logman stop p1_sn -ets 2>&1 | Out-String
# PresentMon was started by this lane with a timed exit. Never kill another session's PID.
$pidFile = "C:\kf\P1\${Tag}_pm.pid"
if (Test-Path $pidFile) {
  $ownPmId = [int](Get-Content $pidFile)
  $ownPm = Get-Process -Id $ownPmId -ErrorAction SilentlyContinue
  if ($ownPm -and $ownPm.StartTime.ToUniversalTime().ToString("o") -eq (Get-Content "C:\kf\P1\${Tag}_pm.start")) {
    Wait-Process -Id $ownPmId -Timeout 20 -ErrorAction Stop
  }
}
Start-Sleep 1
$etl = "C:\kf\P1\$Tag.etl"
if (-not (Test-Path $etl)) { "NO ETL"; exit 1 }
"etl bytes " + (Get-Item $etl).Length
$csv = "C:\kf\P1\${Tag}_dxg.csv"; Remove-Item $csv -ErrorAction SilentlyContinue
$t0 = Get-Date
tracerpt $etl -o $csv -of CSV -y -summary "C:\kf\P1\${Tag}_summary.txt" -report "C:\kf\P1\${Tag}_report.xml" 2>&1 | Out-Null
"tracerpt s " + [int]((Get-Date) - $t0).TotalSeconds + " csv bytes " + (Get-Item $csv -ErrorAction SilentlyContinue).Length
$zip = "C:\kf\P1\$Tag.zip"; Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $csv, "C:\kf\P1\${Tag}_pm.csv" -DestinationPath $zip -CompressionLevel Fastest
"zip bytes " + (Get-Item $zip).Length
& C:\kf\P1\sn_upload.ps1 -File $zip -Name "p1_sn_${Tag}_zip"
