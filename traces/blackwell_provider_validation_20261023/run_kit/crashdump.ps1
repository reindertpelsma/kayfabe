$ErrorActionPreference='Stop'
$k='HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl'
foreach($kv in @(@('CrashDumpEnabled',3),@('AutoReboot',1),@('Overwrite',1),@('LogEvent',1))){ New-ItemProperty -LiteralPath $k -Name $kv[0] -PropertyType DWord -Value $kv[1] -Force | Out-Null }
Get-ItemProperty -LiteralPath $k | Select-Object CrashDumpEnabled,AutoReboot,Overwrite,LogEvent,MinidumpDir | ConvertTo-Json -Compress
$pf=Get-CimInstance Win32_PageFileUsage | Select-Object Name,AllocatedBaseSize; "PAGEFILE $($pf.Name) $($pf.AllocatedBaseSize)"
'CRASHDUMP_READY=1 (small memory dump, effective from the next boot)'
