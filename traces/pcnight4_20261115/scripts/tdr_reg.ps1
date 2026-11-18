# pcnight4 E4: TdrDelay / TdrDdiDelay = 60 (HKLM GraphicsDrivers, DWORD), run through the harness guest-control path (ssh) in a fresh overlay
$k='HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
New-ItemProperty -Path $k -Name TdrDelay -PropertyType DWord -Value 60 -Force | Out-Null
New-ItemProperty -Path $k -Name TdrDdiDelay -PropertyType DWord -Value 60 -Force | Out-Null
Get-ItemProperty $k | Select TdrDelay,TdrDdiDelay,TdrLevel | Format-List
