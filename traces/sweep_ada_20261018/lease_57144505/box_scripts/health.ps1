$ErrorActionPreference='Stop'
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$null=Get-WinEvent -LogName System -MaxEvents 1 -ErrorAction Stop
try { $events=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101,153,1001;StartTime=$boot} -ErrorAction Stop) }
catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*'){throw}; $events=@() }
$gpu=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
$dwm=@(Get-Process dwm -ErrorAction SilentlyContinue)
$explorer=@(Get-Process explorer -ErrorAction SilentlyContinue)
$r=[ordered]@{utc=[DateTime]::UtcNow.ToString('o');boot=$boot.ToUniversalTime().ToString('o');system_log_readable=1;events4101=@($events|Where-Object Id -eq 4101).Count;events153=@($events|Where-Object Id -eq 153).Count;bugcheck1001=@($events|Where-Object Id -eq 1001).Count;driver=$gpu[0].DriverVersion;code=$gpu[0].ConfigManagerErrorCode;dwm=$dwm.Count;explorer=$explorer.Count}
'S2_HEALTH '+($r|ConvertTo-Json -Compress)
$events|Select-Object TimeCreated,Id,ProviderName,Message|ConvertTo-Json -Compress
if($gpu.Count -ne 1 -or $r.driver -ne '32.0.15.8088' -or $r.code -ne 0 -or !$r.dwm -or !$r.explorer -or $events.Count){throw 'S2 desktop/driver/zero-4101-153-bugcheck guard failed'}
'S2_HEALTH_PASS '+[DateTime]::UtcNow.ToString('o')
