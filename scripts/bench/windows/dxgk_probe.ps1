# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxgk_probe.ps1 -- read-only (2026-10-08): what dxgkrnl stored about display paths (the CCD database under
# GraphicsDrivers\Configuration and \Connectivity), and the DxgKrnl/display event channels since boot.
# One line per fact: `DXG <what> <value>`.
$ErrorActionPreference = 'Continue'
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
foreach ($sub in 'Configuration', 'Connectivity', 'ScaleFactors') {
  if (Test-Path "$gd\$sub") {
    Get-ChildItem "$gd\$sub" -Recurse -ErrorAction SilentlyContinue | ForEach-Object {
      $k = $_; $vals = ($k.Property | ForEach-Object { "$_=" + ($k.GetValue($_) -join ',') }) -join '; '
      "DXG reg $($k.Name -replace '^HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\GraphicsDrivers\\','') :: " + ($vals -replace '^(.{0,300}).*','$1')
    }
  } else { "DXG reg $sub absent" }
}
$boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
Get-WinEvent -ListLog *DxgKrnl*, *Display*, *Dwm*, *nvlddmkm* -ErrorAction SilentlyContinue | Where-Object { $_.RecordCount -gt 0 } | ForEach-Object {
  "DXG log $($_.LogName) records=$($_.RecordCount)"
  Get-WinEvent -LogName $_.LogName -MaxEvents 25 -ErrorAction SilentlyContinue | Where-Object { $_.TimeCreated -ge $boot } |
    ForEach-Object { "DXG evt $($_.LogName) $($_.TimeCreated.ToString('HH:mm:ss')) id=$($_.Id) lvl=$($_.LevelDisplayName) :: " + ((($_.Message) -replace '\s+',' ') -replace '^(.{0,200}).*','$1') }
}
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$boot} -MaxEvents 400 -ErrorAction SilentlyContinue |
  Where-Object { $_.LevelDisplayName -in 'Error','Warning','Critical' } | Select-Object -First 30 |
  ForEach-Object { "DXG sys $($_.TimeCreated.ToString('HH:mm:ss')) $($_.ProviderName) id=$($_.Id) lvl=$($_.LevelDisplayName) :: " + ((($_.Message) -replace '\s+',' ') -replace '^(.{0,200}).*','$1') }
