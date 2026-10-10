$k = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
New-ItemProperty -Path $k -Name TdrDelay -PropertyType DWord -Value 30 -Force | Out-Null
New-ItemProperty -Path $k -Name TdrDdiDelay -PropertyType DWord -Value 30 -Force | Out-Null
Get-ItemProperty $k | Select-Object TdrDelay, TdrDdiDelay, TdrLevel, HwSchMode | Format-List | Out-String
