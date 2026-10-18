# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_cap_stop.ps1 -Tag NAME: stop the sngap ETW session and PresentMon, decode the etl with the inbox tracerpt (CSV), zip etl-csv + PresentMon csv,
# and upload the zip to the host kfagent broker (/data/kfagent/r/sngap_NAME_zip.txt). Prints sizes and the HAGS registry value.
param([Parameter(Mandatory = $true)][string]$Tag)
$ErrorActionPreference = 'Continue'
logman stop sngap -ets 2>&1 | Out-String
Get-Process PresentMon* -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 1
$etl = "C:\kf\sn\$Tag.etl"
if (-not (Test-Path $etl)) { "NO ETL"; exit 1 }
"etl bytes " + (Get-Item $etl).Length
$csv = "C:\kf\sn\${Tag}_dxg.csv"; Remove-Item $csv -ErrorAction SilentlyContinue
$t0 = Get-Date
tracerpt $etl -o $csv -of CSV -y -summary "C:\kf\sn\${Tag}_summary.txt" -report "C:\kf\sn\${Tag}_report.xml" 2>&1 | Out-Null
"tracerpt s " + [int]((Get-Date) - $t0).TotalSeconds + " csv bytes " + (Get-Item $csv -ErrorAction SilentlyContinue).Length
$zip = "C:\kf\sn\$Tag.zip"; Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $csv, "C:\kf\sn\${Tag}_pm.csv" -DestinationPath $zip -CompressionLevel Fastest
"zip bytes " + (Get-Item $zip).Length
& C:\kf\sn\sn_upload.ps1 -File $zip -Name "sngap_${Tag}_zip"
