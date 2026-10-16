$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep'
function AdapterCheck {
    $g=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
    if($g.Count -ne 1 -or $g[0].DriverVersion -ne '32.0.15.8088' -or $g[0].ConfigManagerErrorCode -ne 0) { throw 'Hold requires one actual580.88 adapter with Code0' }
    $d=@(Get-Process dwm -ErrorAction SilentlyContinue)
    $e=@(Get-Process explorer -ErrorAction SilentlyContinue)
    if(!$d.Count -or !$e.Count) { throw 'Hold requires DWM and Explorer' }
    Write-Output "HOLD_ADAPTER driver=$($g[0].DriverVersion) code=$($g[0].ConfigManagerErrorCode) dwm=$($d.Count) explorer=$($e.Count)"
}
$dx=Join-Path $root 'kf_dxprobe.exe'
if((Get-FileHash $dx).Hash.ToLower() -ne '2e672bc737a1c6c2332c8b7499cd5459a629d191b27ac4dd9a04a3254c821835') { throw 'D3D probe hash mismatch' }
$null=Get-WinEvent -LogName System -MaxEvents 1 -ErrorAction Stop
AdapterCheck
$since=Get-Date
$clock=[Diagnostics.Stopwatch]::StartNew()
Write-Output "HOLD_START utc=$($since.ToUniversalTime().ToString('o')) requested_seconds=420 repeated_d3d_probes=1"
$round=0
while($clock.Elapsed.TotalSeconds -lt 420) {
    $round++
    foreach($api in 'd3d11','d3d12') {
        & $dx $api --iters 20
        $rc=$LASTEXITCODE
        Write-Output "HOLD_D3D round=$round api=$api rc=$rc elapsed=$([Math]::Round($clock.Elapsed.TotalSeconds,2))"
        if($rc) { throw "D3D hold failed: $api" }
    }
    AdapterCheck
    $remaining=420-$clock.Elapsed.TotalSeconds
    if($remaining -gt 0) { Start-Sleep -Seconds ([Math]::Min(30,[Math]::Ceiling($remaining))) }
}
if($clock.Elapsed.TotalSeconds -lt 420) { throw 'Hold duration too short' }
AdapterCheck
try { $events=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$since} -ErrorAction Stop) }
catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw }; $events=@() }
Write-Output "HOLD_TDR_COUNT=$($events.Count) elapsed_seconds=$([Math]::Round($clock.Elapsed.TotalSeconds,2)) rounds=$round"
if($events.Count) { throw 'TDR during hold' }
Write-Output "HOLD_EXIT rc=0 utc=$([DateTime]::UtcNow.ToString('o'))"
