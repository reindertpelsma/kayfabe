$ErrorActionPreference='Stop'
$root='C:\ProgramData\KayfabeGsp'
$gpu=@(Get-PnpDevice -PresentOnly -Class Display | Select-Object Status,Class,FriendlyName,InstanceId,Problem)
$captures=@(Get-ChildItem "$root\captures" -Directory | ForEach-Object { [ordered]@{name=$_.Name;files=@(Get-ChildItem $_.FullName -File | Select-Object Name,Length)} })
[ordered]@{time=[DateTime]::UtcNow.ToString('o');os=(Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,LastBootUpTime);gpu=$gpu;observer=[string](Get-Service KayfabeGspTrace).Status;observer_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\KayfabeGspTrace').Start;tasks=@(Get-ScheduledTask | Where-Object {$_.TaskName -like '*Kayfabe*' -or $_.TaskName -like '*Vast*'} | Select-Object TaskName,State);deferred=[IO.File]::Exists('C:\ProgramData\VastWindows\defer-native-gpu.flag');reboots_held=[IO.File]::Exists('C:\ProgramData\VastWindows\hold-native-reboots.flag');artifacts=@(Get-ChildItem "$root\build" -File | Select-Object Name,Length);captures=$captures} | ConvertTo-Json -Depth 7
& nvidia-smi.exe -q
