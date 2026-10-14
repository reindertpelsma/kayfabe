$k="HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl"
Set-ItemProperty $k CrashDumpEnabled 1 -Type DWord      # 1 = complete memory dump
Set-ItemProperty $k AlwaysKeepMemoryDump 1 -Type DWord
Set-ItemProperty $k Overwrite 1 -Type DWord
Set-ItemProperty $k AutoReboot 1 -Type DWord
Set-ItemProperty $k LogEvent 1 -Type DWord
$cs=Get-CimInstance Win32_ComputerSystem; $cs.AutomaticManagedPagefile
Get-CimInstance Win32_PageFileUsage | Select Name,AllocatedBaseSize | Format-List
"RAM_MB=" + [int]($cs.TotalPhysicalMemory/1MB)
Get-ItemProperty $k | Select CrashDumpEnabled,AlwaysKeepMemoryDump,Overwrite,AutoReboot | Format-List
(Get-PSDrive C).Free/1GB
