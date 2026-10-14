$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force C:\kfdbg | Out-Null
"uptime_s " + [int]((Get-Date) - (Get-CimInstance Win32_OperatingSystem).LastBootUpTime).TotalSeconds
& wpr.exe -cancel 2>&1 | Out-Null
& wpr.exe -start GPU -filemode 2>&1
"started " + (Get-Date -Format o)
Start-Sleep -Seconds 6
"stopping " + (Get-Date -Format o)
& wpr.exe -stop C:\kfdbg\gpu.etl 2>&1
"stopped " + (Get-Date -Format o)
Get-Item C:\kfdbg\gpu.etl -ErrorAction SilentlyContinue | ForEach-Object { "etl " + $_.Length }
