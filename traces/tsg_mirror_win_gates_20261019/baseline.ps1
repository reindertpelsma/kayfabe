$ErrorActionPreference='Continue'
New-Item -ItemType Directory -Path C:\kf-m2 -ErrorAction Stop | Out-Null
$start=Get-Date
$start.ToString('o') | Set-Content C:\kf-m2\start.txt
"BASELINE_BEGIN $($start.ToString('o'))"
& nvidia-smi
Start-Sleep -Seconds 60
"BASELINE_END $((Get-Date).ToString('o'))"
& nvidia-smi
$events=@(Get-WinEvent -FilterHashtable @{LogName='System';StartTime=$start} -ErrorAction SilentlyContinue | Where-Object {$_.Id -eq 4101 -or $_.ProviderName -match 'nvlddmkm'})
"TDR_EVENTS=$($events.Count)"; $events | Select-Object TimeCreated,Id,ProviderName,Message | Format-List
foreach($d in @('C:\kf\apps','C:\Program Files\Blender Foundation','C:\Users\Public','C:\tools','C:\kf\ogl')) { "DIR $d"; Get-ChildItem $d -ErrorAction SilentlyContinue | Select-Object Name,Mode | Format-Table -AutoSize }
Get-ChildItem C:\kf\apps,'C:\Program Files\Blender Foundation',C:\tools -Recurse -File -ErrorAction SilentlyContinue | Where-Object {$_.Name -match '^(vkcube|vulkaninfo|ffmpeg|ffprobe|python|blender|GravityMark|cup[238]|deviceQuery|vectorAdd|simpleStreams).*\.exe$'} | Select-Object FullName,Length | Format-Table -AutoSize
