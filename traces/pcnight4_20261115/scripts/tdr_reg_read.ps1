$k='HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
Get-ItemProperty $k | Select TdrDelay,TdrDdiDelay,TdrLevel | Format-List
(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
