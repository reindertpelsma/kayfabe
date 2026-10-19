$boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$tdr = @(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$boot} -ErrorAction SilentlyContinue)
$gpu = @(Get-WinEvent -FilterHashtable @{LogName='System';Id=153;StartTime=$boot} -ErrorAction SilentlyContinue | Where-Object ProviderName -eq 'nvlddmkm')
$crashes = @(Get-WinEvent -FilterHashtable @{LogName='Application';Id=1000;StartTime=$boot} -ErrorAction SilentlyContinue)
"HEALTH tdr_4101=$($tdr.Count) gpu_153=$($gpu.Count) app_crashes=$($crashes.Count) boot=$($boot.ToUniversalTime().ToString('o')) user=$((Get-CimInstance Win32_ComputerSystem).UserName)"
