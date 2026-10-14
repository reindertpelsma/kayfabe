# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# edge_probe.ps1 [-Page video] [-Codec h264] [-Seconds 30] [-Out C:\kf\out\edge_probe.txt]
# Native-baseline helper: run kf_edge.ps1 (same page/server as the matrix) and, while the page plays, sample every 2 s
# (a) the msedge.exe processes (pid, parent, --type, --utility-sub-type) and (b) EVERY non-zero GPU Engine counter row
# (per pid, engine type) so the video-decode engine use of the Edge GPU/utility process is visible regardless of the
# process tree. Run it in the interactive session (scheduled task, like kf_launch.ps1 -Mode interactive).
param([string]$Page = 'video', [string]$Codec = 'h264', [int]$Seconds = 30, [string]$Out = 'C:\kf\out\edge_probe.txt')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
'' | Set-Content $Out
$e = Start-Process powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'C:\kf\kf_edge.ps1', '-Id', 'edge_probe', '-Page', $Page, '-Codec', $Codec, '-Seconds', $Seconds) -PassThru -WindowStyle Hidden -RedirectStandardOutput 'C:\kf\out\edge_probe.kfedge.txt'
$t0 = Get-Date
while (-not $e.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt ($Seconds + 70)) {
    Start-Sleep -Seconds 2
    $el = [int]((Get-Date) - $t0).TotalSeconds
    $procs = @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue)
    $pids = @{}
    foreach ($p in $procs) {
        $cl = [string]$p.CommandLine
        $ty = 'browser'; if ($cl -match '--type=([a-z\-]+)') { $ty = $Matches[1] }; if ($cl -match '--utility-sub-type=(\S+)') { $ty += ':' + $Matches[1] }
        $pids[[int]$p.ProcessId] = $ty
        "t=$el proc pid=$($p.ProcessId) ppid=$($p.ParentProcessId) type=$ty" | Add-Content $Out
    }
    try { $cs = (Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples } catch { "t=$el counter error $_" | Add-Content $Out; continue }
    foreach ($c in $cs) {
        if ($c.CookedValue -le 0) { continue }
        if ($c.InstanceName -match '^pid_(\d+)_.*_engtype_(.+)$') {
            $q = [int]$Matches[1]; $et = $Matches[2]
            $who = 'other'; if ($pids.ContainsKey($q)) { $who = 'msedge/' + $pids[$q] } else { try { $who = (Get-Process -Id $q -ErrorAction Stop).ProcessName } catch { } }
            "t=$el eng pid=$q ($who) engtype=$et util=$([math]::Round($c.CookedValue,2))" | Add-Content $Out
        }
    }
}
'--- kfedge ---' | Add-Content $Out
Get-Content 'C:\kf\out\edge_probe.kfedge.txt' -ErrorAction SilentlyContinue | Add-Content $Out
'DONE' | Add-Content $Out
