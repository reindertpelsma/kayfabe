# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# guest_logs_light.ps1 (read-only, ~1-3 s): one timeline sample. Run through QGA guest-exec as SYSTEM by gl-monitor.py.
# Prints one machine line LS (GPU problem code, NVIDIA service states, container process) plus short detail.
$ErrorActionPreference='Continue'
$os=Get-CimInstance Win32_OperatingSystem
'LIGHT utc=' + (Get-Date).ToUniversalTime().ToString('HH:mm:ss.fff') + ' boot=' + $os.LastBootUpTime.ToString('HH:mm:ss') + ' up_s=' + [int]((Get-Date)-$os.LastBootUpTime).TotalSeconds
$v=@(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'NVIDIA' })
$g='none'; if($v.Count){ $g=($v | ForEach-Object { 'code' + $_.ConfigManagerErrorCode + '/' + $_.Status }) -join ',' }
$sv=@(Get-CimInstance Win32_Service -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^NV|NVIDIA' -or $_.DisplayName -match 'NVIDIA' })
$ss=($sv | ForEach-Object { $_.Name + '=' + $_.State + '(pid' + $_.ProcessId + ')' }) -join ';'
$pr=@(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^(NVDisplay|nvcontainer|nvidia|NVIDIA|nvspcap|nvvsvc|dwm|LogonUI|winlogon|csrss)' })
$ps=($pr | ForEach-Object { $_.Name + ':' + $_.Id + ':cpu' + [int]$_.CPU + ':h' + $_.HandleCount }) -join ' '
'LS gpu=' + $g + ' svc=' + $ss + ' proc=' + $ps
$sv | Select-Object Name,State,StartMode,ProcessId,ExitCode | Format-Table -AutoSize | Out-String -Width 200
