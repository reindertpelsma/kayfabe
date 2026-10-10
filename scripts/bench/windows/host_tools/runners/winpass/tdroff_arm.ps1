# winpass (diagnostic, guest-side): TdrLevel = 0 (Windows' GPU timeout detection OFF) for the NEXT boot, so a
# stall is held instead of torn down and can be inspected live; then arm the boot-time DxgKrnl ETW session
# (Base keyword, as dxg_etw_arm.ps1). Nothing outside the guest is touched.
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
Set-ItemProperty -Path $gd -Name TdrLevel -Type DWord -Value 0
'TDROFF TdrLevel=' + (Get-ItemProperty $gd).TdrLevel
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
logman delete "autosession\kfdxg" 2>&1 | Out-Null
$o = logman create trace "autosession\kfdxg" -p "Microsoft-Windows-DxgKrnl" 0x1 5 -o C:\kf\kfdxg.etl -bs 1024 -nb 64 512 -ft 1 -max 512 2>&1
'TDROFF etw ' + ($o -join ' ')
