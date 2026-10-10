# winpass run91 variable: HwSchMode = 1 (hardware-accelerated GPU scheduling OFF) for the next boot
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
Set-ItemProperty -Path $gd -Name HwSchMode -Type DWord -Value 1
'HWSCH HwSchMode=' + (Get-ItemProperty $gd).HwSchMode
