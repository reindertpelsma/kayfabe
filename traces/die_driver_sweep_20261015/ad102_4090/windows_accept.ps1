$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep'
$since=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
Write-Output "WIN_ACCEPT_START $([DateTime]::UtcNow.ToString('o'))"
$gpu=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
if($gpu.Count -ne 1) {throw 'Expected one NVIDIA display adapter'}
$gpu | Select-Object Name,DriverVersion,Status,ConfigManagerErrorCode | ConvertTo-Json -Compress
if($gpu[0].ConfigManagerErrorCode -ne 0 -or $gpu[0].DriverVersion -ne '32.0.15.8088') {throw 'Adapter or expected driver check failed'}
$dwm=@(Get-Process dwm -ErrorAction SilentlyContinue)
$desktop=@(Get-Process explorer -ErrorAction SilentlyContinue)
Write-Output "WIN_DESKTOP dwm=$($dwm.Count) explorer=$($desktop.Count)"
if(!$dwm.Count -or !$desktop.Count) {throw 'DWM and signed-in desktop required'}
$files=@{
 'kf_dxprobe.exe'='2e672bc737a1c6c2332c8b7499cd5459a629d191b27ac4dd9a04a3254c821835'
 'cup2.exe'='eba68ac8f92cf1d65793a6df70d62624d33a8479b3137a45d07d7fd093d79798'
 'cup3.exe'='5df837a1914b0fef34e724605b0494c95788a5676f1420863a441a923d0f3f87'
 'cup8.exe'='00e69ceb25d4d3af697a79a1bf133fc5f02f5e9125eefde288fa371d856cdef8'
}
foreach($name in $files.Keys) {
 if((Get-FileHash (Join-Path $root $name)).Hash.ToLower() -ne $files[$name]) {throw "Probe hash mismatch: $name"}
}
foreach($name in 'cup2','cup3','cup8') {
 & "$root\$name.exe"
 $rc=$LASTEXITCODE
 Write-Output "WIN_CUDA $name rc=$rc"
 if($rc) {throw "$name failed"}
}
foreach($api in 'd3d11','d3d12') {
 & "$root\kf_dxprobe.exe" $api --iters 20
 $rc=$LASTEXITCODE
 Write-Output "WIN_D3D $api rc=$rc"
 if($rc) {throw "$api failed"}
}
$tdr=@(Get-WinEvent -FilterHashtable @{LogName='System'; Id=4101; StartTime=$since} -ErrorAction SilentlyContinue)
Write-Output "WIN_TDR_SINCE_BOOT=$($tdr.Count)"
if($tdr.Count) {throw 'TDR recorded since this boot'}
Write-Output "WIN_ACCEPT_EXIT rc=0 $([DateTime]::UtcNow.ToString('o'))"
