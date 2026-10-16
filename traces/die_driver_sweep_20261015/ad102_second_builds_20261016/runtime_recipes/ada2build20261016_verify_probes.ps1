$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep'
foreach($name in 'cup2.exe','cup3.exe','cup8.exe','kf_dxprobe.exe') {
$p=Join-Path $root $name
if(Test-Path $p) { Write-Output "PROBE_FILE name=$name bytes=$((Get-Item $p).Length) sha256=$((Get-FileHash $p).Hash.ToLower())" } else { Write-Output "PROBE_FILE_MISSING name=$name" }
}
Write-Output "INSTALL_STARTED_MARKER=$([bool](Test-Path (Join-Path $root 'driver58151.started')))"
