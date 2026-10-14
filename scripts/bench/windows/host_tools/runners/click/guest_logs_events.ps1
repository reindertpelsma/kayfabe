# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# guest_logs_events.ps1 (read-only, ~2-4 s): the live event log since boot, plus the newest lines of the NVIDIA logs. Run every ~15 s
# before the death so the events the guest has not flushed to disk are read from the running Event Log service.
$ErrorActionPreference='Continue'
function T($d){ if($d){ $d.ToUniversalTime().ToString('HH:mm:ss.fff') } else { '-' } }
function EV($e){ $m=''; try { $m=($e.Message -replace '\s+',' ') } catch {}; if($m.Length -gt 200){ $m=$m.Substring(0,200) }; (T $e.TimeCreated) + ' ' + $e.ProviderName + ' id=' + $e.Id + ' L' + $e.Level + ' ' + $m }
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
'EVENTS utc=' + (Get-Date).ToUniversalTime().ToString('HH:mm:ss.fff') + ' boot=' + (T $boot)
$rx='nvlddmkm|Display|DxgKrnl|Kernel-Power|WER-SystemErrorReporting|Service Control Manager|NVIDIA|Application Error|Windows Error Reporting|LiveKernel|BugCheck|WHEA|volmgr|EventLog|Kernel-PnP'
$all=@(Get-WinEvent -FilterHashtable @{LogName='System','Application'; StartTime=$boot} -MaxEvents 3000 -ErrorAction SilentlyContinue)
'total=' + $all.Count + ' newest=' + $(if($all.Count){ T $all[0].TimeCreated }else{'-'})
@($all | Where-Object { $_.Level -in 1,2,3 -or $_.ProviderName -match $rx } | Sort-Object TimeCreated | Select-Object -Last 60) | ForEach-Object { EV $_ }
'--- DxgKrnl-Operational'
@(Get-WinEvent -LogName 'Microsoft-Windows-DxgKrnl-Operational' -MaxEvents 10 -ErrorAction SilentlyContinue) | ForEach-Object { EV $_ }
'--- LiveKernelReports'
Get-ChildItem 'C:\Windows\LiveKernelReports' -Recurse -Force -ErrorAction SilentlyContinue | Select-Object -First 10 | ForEach-Object { (T $_.LastWriteTime) + ' ' + $_.Length + ' ' + $_.FullName }
'--- nvtopps.log tail 12'
Get-Content -LiteralPath 'C:\ProgramData\NVIDIA Corporation\nvtopps\nvtopps.log' -Tail 12 -ErrorAction SilentlyContinue | ForEach-Object { if($_.Length -gt 200){ $_.Substring(0,200) } else { $_ } }
'--- NVDisplay.ContainerLocalSystem.log size/tail 6'
$cl=Get-Item -LiteralPath 'C:\ProgramData\NVIDIA\NVDisplay.ContainerLocalSystem.log' -ErrorAction SilentlyContinue
if($cl){ 'size=' + $cl.Length + ' mtime=' + (T $cl.LastWriteTime) }
Get-Content -LiteralPath 'C:\ProgramData\NVIDIA\NVDisplay.ContainerLocalSystem.log' -Tail 6 -ErrorAction SilentlyContinue | ForEach-Object { if($_.Length -gt 200){ $_.Substring(0,200) } else { $_ } }
'=== END utc=' + (Get-Date).ToUniversalTime().ToString('HH:mm:ss.fff')
