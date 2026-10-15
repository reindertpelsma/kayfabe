$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
Set-Location C:\ProgramData\KayfabeSweep
$since=Get-Date
$since.ToUniversalTime().ToString('o') | Set-Content oracle.start
Write-Output "NATIVE_ORACLE_START $($since.ToUniversalTime().ToString('o'))"
Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,Status,ConfigManagerErrorCode | ConvertTo-Json -Compress
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
$tdr=@(Get-WinEvent -FilterHashtable @{LogName='System'; Id=4101; StartTime=$since} -ErrorAction SilentlyContinue)
Write-Output "NATIVE_TDR_COUNT=$($tdr.Count)"
if($tdr.Count) { throw 'TDR during oracle' }
Write-Output "NATIVE_ORACLE_EXIT rc=0 $([DateTime]::UtcNow.ToString('o'))"
