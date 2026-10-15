$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
Set-Location C:\ProgramData\KayfabeSweep
$since=Get-Date
$since.ToUniversalTime().ToString('o') | Set-Content oracle.start
Write-Output "NATIVE_ORACLE_START $($since.ToUniversalTime().ToString('o'))"
$gpu=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
$gpu | Select-Object Name,DriverVersion,Status,ConfigManagerErrorCode | ConvertTo-Json -Compress
if($gpu.Count -ne 1 -or $gpu[0].ConfigManagerErrorCode -ne 0) { throw 'Need exactly one healthy NVIDIA adapter' }
if($gpu[0].DriverVersion -ne '32.0.15.8088') { throw 'Native oracle requires actual driver580.88' }
$hashes=@{
 'kf_dxprobe.exe'='2e672bc737a1c6c2332c8b7499cd5459a629d191b27ac4dd9a04a3254c821835'
 'cuda\cup2.exe'='eba68ac8f92cf1d65793a6df70d62624d33a8479b3137a45d07d7fd093d79798'
 'cuda\cup3.exe'='5df837a1914b0fef34e724605b0494c95788a5676f1420863a441a923d0f3f87'
 'cuda\cup8.exe'='00e69ceb25d4d3af697a79a1bf133fc5f02f5e9125eefde288fa371d856cdef8'
}
foreach($name in $hashes.Keys) {
 if((Get-FileHash -Algorithm SHA256 -LiteralPath $name).Hash.ToLower() -ne $hashes[$name]) { throw "Probe hash mismatch: $name" }
 Write-Output "NATIVE_PROBE_HASH_OK=$name"
}
$null=Get-WinEvent -LogName System -MaxEvents 1 -ErrorAction Stop
Write-Output 'NATIVE_SYSTEM_LOG_READABLE=1'
& nvidia-smi.exe --query-gpu=name,driver_version,driver_model.current,memory.total --format=csv,noheader
if($LASTEXITCODE) { throw 'nvidia-smi failed' }
& nvidia-smi.exe -q | Select-String 'GSP Firmware Version'
foreach($name in 'cup2','cup3','cup8') {
 & ".\cuda\$name.exe"
 Write-Output "NATIVE_CUDA $name rc=$LASTEXITCODE"
 if($LASTEXITCODE) { throw "$name failed" }
}
foreach($api in 'd3d11','d3d12') {
 & .\kf_dxprobe.exe $api --iters 20
 Write-Output "NATIVE_D3D $api rc=$LASTEXITCODE"
 if($LASTEXITCODE) { throw "$api failed" }
}
try { $tdr=@(Get-WinEvent -FilterHashtable @{LogName='System'; Id=4101; StartTime=$since} -ErrorAction Stop) }
catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw }; $tdr=@() }
Write-Output "NATIVE_TDR_COUNT=$($tdr.Count)"
if($tdr.Count) { throw 'TDR during oracle' }
$after=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
if($after.Count -ne 1 -or $after[0].DriverVersion -ne '32.0.15.8088' -or $after[0].ConfigManagerErrorCode -ne 0) { throw 'Adapter/build changed during oracle' }
Write-Output 'NATIVE_POST_DRIVER580_CODE0=1'
Write-Output "NATIVE_ORACLE_EXIT rc=0 $([DateTime]::UtcNow.ToString('o'))"
