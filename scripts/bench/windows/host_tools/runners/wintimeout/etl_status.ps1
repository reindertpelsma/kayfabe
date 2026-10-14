"uptime_s " + [int]((Get-Date) - (Get-CimInstance Win32_OperatingSystem).LastBootUpTime).TotalSeconds
Get-ChildItem C:\kfdbg\*.etl, C:\Windows\Minidump\* -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName + " " + $_.Length + " " + $_.LastWriteTime }
& wpr.exe -status 2>&1 | Select-Object -First 5
