$ErrorActionPreference='Stop'
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$null=Get-WinEvent -LogName System -MaxEvents 1 -ErrorAction Stop
try { $events=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101,153,1001;StartTime=$boot} -ErrorAction Stop) }
catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*'){throw}; $events=@() }
$gpu=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
$dwm=@(Get-Process dwm -ErrorAction SilentlyContinue)
$explorer=@(Get-Process explorer -ErrorAction SilentlyContinue)
$user=(Get-CimInstance Win32_ComputerSystem).UserName
$desktop=$null
if($explorer.Count){$desktop=($explorer | Sort-Object StartTime | Select-Object -First 1).StartTime}
$boot153=@($events | Where-Object { $_.Id -eq 153 -and $_.ProviderName -eq 'Microsoft-Windows-Kernel-Boot' })
$now=[DateTime]::UtcNow
$bootPre=@($boot153 | Where-Object { $desktop -and $_.TimeCreated -le $desktop })
$bootPost=@($boot153 | Where-Object { $desktop -and $_.TimeCreated -gt $desktop -and $_.TimeCreated -le $now })
$bootFuture=@($boot153 | Where-Object { $_.TimeCreated -gt $now })
$faults=@($events | Where-Object { $_ -notin $boot153 })
$r=[ordered]@{utc=$now.ToString('o');boot=$boot.ToUniversalTime().ToString('o');desktop_start=$(if($desktop){$desktop.ToUniversalTime().ToString('o')}else{''});user=$user;system_log_readable=1;events4101=@($faults|Where-Object Id -eq 4101).Count;events153=@($faults|Where-Object { $_.Id -eq 153 -and $_.ProviderName -match 'nvlddmkm' }).Count;unclassified153=@($faults|Where-Object { $_.Id -eq 153 -and $_.ProviderName -notmatch 'nvlddmkm' }).Count;boot153_pre_desktop=$bootPre.Count;boot153_post_desktop=$bootPost.Count;boot153_future_clock=$bootFuture.Count;bugcheck1001=@($faults|Where-Object Id -eq 1001).Count;driver=$gpu[0].DriverVersion;code=$gpu[0].ConfigManagerErrorCode;dwm=$dwm.Count;explorer=$explorer.Count}
'S2_HEALTH '+($r|ConvertTo-Json -Compress)
$events|Select-Object TimeCreated,Id,ProviderName,Message|ConvertTo-Json -Compress
if($gpu.Count -ne 1 -or $r.driver -ne '32.0.15.8088' -or $r.code -ne 0 -or !$r.dwm -or !$r.explorer -or !$user -or $faults.Count){throw 'S2 desktop/driver/zero-4101-153-bugcheck guard failed'}
'S2_HEALTH_PASS '+[DateTime]::UtcNow.ToString('o')
