# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_cap_start.ps1 -Tag NAME [-Keywords 0x1] [-Seconds 100]: LIVE (not boot-time) capture, run as SYSTEM through the kfagent diag channel.
#   1. ETW session `p1_sn` on Microsoft-Windows-DxgKrnl (keyword 0x1 = Base: DmaPacket/QueuePacket/context status/VSync...), level 5,
#      QPC clock (-ct perf), C:\kf\P1\NAME.etl, bounded to 400 MiB. Nothing is armed for the next boot (no autologger, no TdrDelay).
#   2. PresentMon 2.6.0 (already on the guest disk) for every presenting process, --qpc_time, --timed Seconds -> C:\kf\P1\NAME_pm.csv
param([Parameter(Mandatory = $true)][string]$Tag, [string]$Keywords = '0x1', [int]$Seconds = 100)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force C:\kf\P1 | Out-Null
logman stop p1_sn -ets 2>&1 | Out-Null
$etl = "C:\kf\P1\$Tag.etl"; Remove-Item $etl -ErrorAction SilentlyContinue
$o = logman create trace p1_sn -ets -p Microsoft-Windows-DxgKrnl $Keywords 5 -o $etl -ct perf -bs 128 -nb 32 256 -ft 1 -max 400 2>&1
"ETW create: " + ($o -join ' ')
$pm = "C:\kf\PresentMon-2.6.0-x64.exe"
$ownPm = Start-Process -PassThru -FilePath $pm -ArgumentList @("--output_file", "C:\kf\P1\${Tag}_pm.csv", "--session_name", "p1_sn_pm", "--stop_existing_session", "--no_console_stats", "--qpc_time", "--timed", $Seconds, "--terminate_after_timed") -WindowStyle Hidden -RedirectStandardError "C:\kf\P1\${Tag}_pm.err" -RedirectStandardOutput "C:\kf\P1\${Tag}_pm.out"
"CAPSTART " + (Get-Date).ToUniversalTime().ToString('o') + " qpc=" + [Diagnostics.Stopwatch]::GetTimestamp() + " freq=" + [Diagnostics.Stopwatch]::Frequency

$ownPm.Id | Set-Content "C:\kf\P1\${Tag}_pm.pid"
$ownPm.StartTime.ToUniversalTime().ToString("o") | Set-Content "C:\kf\P1\${Tag}_pm.start"
