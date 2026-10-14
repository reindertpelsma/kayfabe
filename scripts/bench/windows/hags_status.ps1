# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# winpass: the guest's hardware-scheduling (HAGS) state and display adapters, from dxdiag (read-only)
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
Remove-Item C:\kf\dxdiag.txt -ErrorAction SilentlyContinue
$p = Start-Process dxdiag.exe -ArgumentList '/t C:\kf\dxdiag.txt' -PassThru
$p.WaitForExit(120000) | Out-Null
for ($i=0; $i -lt 60 -and -not (Test-Path C:\kf\dxdiag.txt); $i++) { Start-Sleep 1 }
Select-String -Path C:\kf\dxdiag.txt -Pattern 'Card name|Hardware Scheduling|Driver Version|Feature Levels|Driver Model|Device Problem|Current Mode|Monitor Name|Output Type|DDI Version' | ForEach-Object { 'HAGS ' + $_.Line.Trim() }
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
'HAGS HwSchMode=' + (Get-ItemProperty $gd).HwSchMode + ' TdrDelay=' + (Get-ItemProperty $gd).TdrDelay + ' TdrDdiDelay=' + (Get-ItemProperty $gd).TdrDdiDelay
Get-PnpDevice -Class Display | ForEach-Object { 'HAGS PNP ' + $_.FriendlyName + ' ' + $_.Status + ' ' + $_.Problem }
