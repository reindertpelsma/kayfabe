# post_idle.ps1 -- run through QGA (SYSTEM) AFTER the idle window, before the guest-log collector. Read-only, except that
# nvidia-smi itself issues RM controls (the coordinator asked for it as the liveness check).
$ErrorActionPreference='Continue'
'POSTIDLE utc=' + (Get-Date).ToUniversalTime().ToString('HH:mm:ss.fff')
'--- VideoController'
Get-CimInstance Win32_VideoController | Select-Object Name,ConfigManagerErrorCode,Status | Format-List | Out-String -Width 200
'--- PnP Display+Monitor'
Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.Class -in 'Display','Monitor' } | ForEach-Object { $pp=@(Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName 'DEVPKEY_Device_ProblemCode' -ErrorAction SilentlyContinue); '{0} | {1} | {2} | status={3} problem={4}' -f $_.Class,$_.FriendlyName,$_.InstanceId,$_.Status,($pp | Select-Object -First 1).Data }
'--- nvidia-smi'
$smi = 'C:\Windows\System32\nvidia-smi.exe'
if(-not (Test-Path $smi)){ $smi = (Get-ChildItem 'C:\Windows\System32\DriverStore\FileRepository\nv_dispi*\nvidia-smi.exe' -ErrorAction SilentlyContinue | Select-Object -First 1).FullName }
if($smi){ & $smi 2>&1 | Out-String -Width 200; 'nvidia-smi exit=' + $LASTEXITCODE } else { 'nvidia-smi not found' }
'--- nvlddmkm events (System, newest 12) and LiveDump (newest 5)'
Get-WinEvent -FilterHashtable @{LogName='System'; ProviderName='nvlddmkm'} -MaxEvents 12 -ErrorAction SilentlyContinue | ForEach-Object { $_.TimeCreated.ToUniversalTime().ToString('HH:mm:ss.fff') + ' id=' + $_.Id }
Get-WinEvent -LogName 'Microsoft-Windows-Kernel-LiveDump/Operational' -MaxEvents 5 -ErrorAction SilentlyContinue | ForEach-Object { 'LiveDump ' + $_.TimeCreated.ToUniversalTime().ToString('HH:mm:ss.fff') + ' id=' + $_.Id }
'=== END'
