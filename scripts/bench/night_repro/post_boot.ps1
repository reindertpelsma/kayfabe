# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([string]$Root, [int]$Seconds=720, [string]$Pair='heaven-gravity')
$ErrorActionPreference='Stop'
. "$PSScriptRoot\transport.ps1"
New-Item -ItemType Directory -Force $Root | Out-Null
$pm='C:\kf\hang\pm_live.csv'; $trig='C:\kf\hang\trig.log'
$meta=@{frequency=[Diagnostics.Stopwatch]::Frequency; start_qpc=[Diagnostics.Stopwatch]::GetTimestamp();
    pm_offset=0; trig_offset=0; pm_header=''; root=$Root; pair=$Pair; start_utc=(Get-Date).ToUniversalTime().ToString('o')}
if(Test-Path $pm) { $meta.pm_offset=(Get-Item $pm).Length; $meta.pm_header=Get-Content $pm -TotalCount 1 }
if(Test-Path $trig) { $meta.trig_offset=(Get-Item $trig).Length }
(ConvertTo-N1Json $meta) | Set-Content "$Root\start.json"
& logman stop hangetw -ets 2>&1 | Out-Null
& logman create trace hangetw -ets -p Microsoft-Windows-DxgKrnl 0x1 5 -o C:\kf\hang\cur.etl -ct perf -f bincirc -max 400 -bs 128 -nb 32 256 2>&1 | Out-Null
if($LASTEXITCODE -ne 0) { throw 'DxgKrnl session creation failed' }
foreach($name in @('kf-vcpuwatch','kf-pmmon')) { if((Get-ScheduledTask $name).State -ne 'Running') { Start-ScheduledTask $name } }
$principal=(Get-ScheduledTask kfogl-heaven_d3d11_win).Principal
$clients=switch($Pair) { 'heaven-heaven' {@('Heaven','Heaven2')}; 'heaven-blender' {@('Heaven','Blender')}; 'heaven-edge' {@('Heaven','Edge')}; default {@('Heaven','GravityMark')} }
foreach($client in $clients) {
    $a=New-ScheduledTaskAction -Execute powershell.exe -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File C:\kf\n1\load.ps1 -Client $client -Root $Root -Seconds $Seconds -Pair $Pair"
    $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 20)
    Register-ScheduledTask -TaskName "kf-n1-$client" -Action $a -Principal $principal -Settings $settings -Force | Out-Null
    Start-ScheduledTask "kf-n1-$client"
}
'POST_BOOT_OK'
ConvertTo-N1Json @(Get-CimInstance Win32_VideoController | Select-Object Name,CurrentHorizontalResolution,CurrentVerticalResolution,ConfigManagerErrorCode)
