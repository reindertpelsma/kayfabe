# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# guest_logs_collect.ps1 (READ-ONLY, size-limited): the Windows guest's own view of its NVIDIA driver, services, processes,
# event logs and dump/log directories. Run through QGA guest-exec (SYSTEM) by gl-monitor.py at moments A (driver up),
# B (driver died), C (+60 s). Writes nothing. No nvidia-smi (it would issue RM controls and perturb the guest).
$ErrorActionPreference='Continue'
function T($d){ if($d){ $d.ToUniversalTime().ToString('HH:mm:ss.fff') } else { '-' } }
function S($t,[scriptblock]$b,[int]$max=300){ '=== ' + $t; try { @(& $b | Out-String -Width 250 -Stream | Where-Object { $_.Trim() }) | Select-Object -First $max } catch { 'ERR ' + $_.Exception.Message } }
function EV($e){ $m=''; try { $m=($e.Message -replace '\s+',' ') } catch {}; if($m.Length -gt 200){ $m=$m.Substring(0,200) }; (T $e.TimeCreated) + ' ' + $e.ProviderName + ' id=' + $e.Id + ' L' + $e.Level + ' ' + $m }
function D($p,[int]$n=80){ Get-ChildItem -Path $p -Force -Recurse -Depth 2 -ErrorAction SilentlyContinue | Select-Object -First $n | ForEach-Object { (T $_.LastWriteTime) + ' ' + $(if($_.PSIsContainer){'<DIR>'}else{$_.Length}) + ' ' + $_.FullName } }
$os=Get-CimInstance Win32_OperatingSystem; $boot=$os.LastBootUpTime
'GUESTLOGS utc=' + (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd HH:mm:ss.fff') + ' boot_utc=' + (T $boot) + ' up_s=' + [int]((Get-Date)-$boot).TotalSeconds + ' whoami=' + (whoami) + ' build=' + $os.BuildNumber
S 'SERVICES (Win32_Service: NV*/NVIDIA*)' { Get-CimInstance Win32_Service | Where-Object { $_.Name -match 'NV|NVIDIA' -or $_.DisplayName -match 'NVIDIA' } | Select-Object Name,DisplayName,State,StartMode,DelayedAutoStart,ExitCode,ProcessId,StartName,PathName | Format-List } 120
S 'SERVICES (Get-Service named)' { Get-Service -Name 'NVDisplay.ContainerLocalSystem','NVDisplay.Container LS','NvContainerLocalSystem','NVIDIA*' -ErrorAction SilentlyContinue | Select-Object Name,Status,StartType,ServiceType | Format-Table -AutoSize } 40
S 'SERVICE PROCESS START TIMES' { Get-CimInstance Win32_Service | Where-Object { ($_.Name -match 'NV|NVIDIA' -or $_.DisplayName -match 'NVIDIA') -and $_.ProcessId } | ForEach-Object { $p=Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue; $_.Name + ' pid=' + $_.ProcessId + ' start_utc=' + $(if($p){ T $p.StartTime }else{'gone'}) } } 40
S 'PROCESSES' { Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^(NVDisplay|nvcontainer|nvidia|NVIDIA|nvspcap|nvvsvc|NvTelemetry|nvlddmkm|dwm|LogonUI|winlogon|csrss)' } | ForEach-Object { '{0,-22} pid={1,-5} sess={2} cpu_s={3,-8} handles={4,-5} threads={5,-4} start_utc={6} responding={7}' -f $_.Name,$_.Id,$_.SI,[math]::Round([double]$_.CPU,2),$_.HandleCount,$_.Threads.Count,(T $_.StartTime),$_.Responding } } 60
S 'PNP DISPLAY/NVIDIA' { Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.Class -eq 'Display' -or $_.FriendlyName -match 'NVIDIA' } | ForEach-Object { $pp=@(Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName 'DEVPKEY_Device_ProblemCode','DEVPKEY_Device_ProblemStatus' -ErrorAction SilentlyContinue); '{0} | {1} | {2} | status={3} problem={4} problemstatus=0x{5:x}' -f $_.Class,$_.FriendlyName,$_.InstanceId,$_.Status,($pp | Where-Object KeyName -match 'ProblemCode').Data,[int](($pp | Where-Object KeyName -match 'ProblemStatus').Data) } } 40
S 'VIDEOCONTROLLER' { Get-CimInstance Win32_VideoController | Select-Object Name,ConfigManagerErrorCode,Status,DriverVersion,PNPDeviceID | Format-List } 40
$all=@(Get-WinEvent -FilterHashtable @{LogName='System','Application'; StartTime=$boot} -MaxEvents 4000 -ErrorAction SilentlyContinue)
'=== EVENTS System+Application since boot (' + (T $boot) + ' UTC): total=' + $all.Count + '; Level 1-3 plus providers nvlddmkm|Display|DxgKrnl|Kernel-Power|WER-SystemErrorReporting|Service Control Manager|NVIDIA|Application Error|Windows Error Reporting|LiveKernel; oldest first, max 300'
$rx='nvlddmkm|Display|DxgKrnl|Kernel-Power|WER-SystemErrorReporting|Service Control Manager|NVIDIA|Application Error|Windows Error Reporting|LiveKernel'
$sel=@($all | Where-Object { $_.Level -in 1,2,3 -or $_.ProviderName -match $rx } | Sort-Object TimeCreated)
'selected=' + $sel.Count
$sel | Select-Object -First 300 | ForEach-Object { EV $_ }
foreach($ln in 'Microsoft-Windows-DxgKrnl-Operational','Microsoft-Windows-Kernel-LiveDump/Operational','Microsoft-Windows-Kernel-PnP/Configuration'){
  '=== LOG ' + $ln + ' (newest 40, newest first)'
  $r=@(Get-WinEvent -LogName $ln -MaxEvents 40 -ErrorAction SilentlyContinue)
  if($r.Count){ $r | ForEach-Object { EV $_ } } else { '(none or log absent)' }
}
S 'DIR C:\Windows\LiveKernelReports' { D 'C:\Windows\LiveKernelReports\*' 80 } 80
S 'DIR C:\Windows\Minidump' { D 'C:\Windows\Minidump\*' 40 } 40
S 'DIR C:\ProgramData\NVIDIA Corporation' { D 'C:\ProgramData\NVIDIA Corporation\*' 80 } 80
S 'DIR C:\ProgramData\NVIDIA' { D 'C:\ProgramData\NVIDIA\*' 60 } 60
'=== NVIDIA log tails (newest 6 *.log under ProgramData\NVIDIA*, systemprofile and user LocalAppData NVIDIA, Windows\Temp nv*.log; last 40 lines each, 240 chars/line)'
$c=@()
foreach($p in 'C:\ProgramData\NVIDIA','C:\ProgramData\NVIDIA Corporation','C:\Windows\System32\config\systemprofile\AppData\Local\NVIDIA','C:\Users\*\AppData\Local\NVIDIA'){ $c+=@(Get-ChildItem -Path $p -Filter *.log -Recurse -Depth 3 -Force -ErrorAction SilentlyContinue) }
$c+=@(Get-ChildItem -Path 'C:\Windows\Temp' -Filter 'nv*.log' -Force -ErrorAction SilentlyContinue)
'candidates=' + $c.Count
$c | Sort-Object LastWriteTime -Descending | Select-Object -First 6 | ForEach-Object {
  '--- ' + $_.FullName + ' size=' + $_.Length + ' mtime_utc=' + (T $_.LastWriteTime)
  try { Get-Content -LiteralPath $_.FullName -Tail 40 -ErrorAction Stop | ForEach-Object { if($_.Length -gt 240){ $_.Substring(0,240) } else { $_ } } } catch { 'ERR ' + $_.Exception.Message }
}
'=== END utc=' + (Get-Date).ToUniversalTime().ToString('HH:mm:ss.fff')
