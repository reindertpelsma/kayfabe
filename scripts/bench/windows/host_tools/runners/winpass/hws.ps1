$gd='HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
Get-ItemProperty $gd | Format-List | Out-String -Width 200
Get-ChildItem 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers\Scheduler' -ErrorAction SilentlyContinue | Out-String
Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers\Scheduler' -ErrorAction SilentlyContinue | Format-List | Out-String
Get-ChildItem C:\kf -ErrorAction SilentlyContinue | Select-Object Name,Length | Out-String
