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
 'kf_dxprobe.exe'='8f049e04196db99275d901119e9c860058d423e2250920acb717062ed8303f3a'
 'cup2.exe'='86039b888dc7d17b1bd92736c5a5589a4a67a897a5aa9859f7a7ed9d280d95b8'
 'cup3.exe'='41681173e42c40ba94cafd33be22c90f419b64436b9be88993ba6bd286a19460'
 'cup8.exe'='7401a67a58288e0789a8fa214be4696316715c5306b690781996847f5baffcf6'
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
try { $tdr=@(Get-WinEvent -FilterHashtable @{LogName='System'; Id=4101; StartTime=$since} -ErrorAction Stop) } catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw }; $tdr=@() }
Write-Output "WIN_TDR_SINCE_BOOT=$($tdr.Count)"
if($tdr.Count) {throw 'TDR recorded since this boot'}
Write-Output "WIN_ACCEPT_EXIT rc=0 $([DateTime]::UtcNow.ToString('o'))"
