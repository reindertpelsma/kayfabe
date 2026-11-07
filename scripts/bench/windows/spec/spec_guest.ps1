# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# spec_guest.ps1 -- the guest side of scripts/bench/box/specviewperf_stage.sh, one action per call. Every call
# is a short ssh command (the long-running CLI is started DETACHED by `launch` as a one-shot interactive
# scheduled task, so a hung or reset-stalled run cannot hold the ssh session).
#
#   -Action prep                         C:\spec\out, Defender path exclusions (best effort), free disk
#   -Action post                         write access for the run user on the tree, the shipped
#                                        vcredist_x64.exe, the CLI's presence
#   -Action verify                       `VIEWSET <dir> files=<n> bytes=<n>` per viewset directory held
#   -Action launch -Workload W           start spec_run.ps1 in the console user's desktop session
#   -Action status -Workload W           `SPEC_STATE end=<0|1> running=<0|1>`
#   -Action collect -Workload W          the status-only files of the run, between BEGIN/END markers
#   -Action kill                         stop a running CLI and its task
#
# STATUS: CODE, 2026-11-07 (docs/design/V3_SPEC_GATE.md). NOT RUN on Windows: written against the
# 2026-11-05 and 2026-11-07 evidence (traces/specviewperf_vast_20261105/README.md section 3,
# traces/specviewperf_native_20261107/README.md); the first box run decides. It never opens the
# vendor result directory, so no score is read, printed or copied.
param(
    [Parameter(Mandatory = $true)][ValidateSet('prep', 'post', 'verify', 'launch', 'status', 'collect', 'kill')][string]$Action,
    [string]$Workload = '',
    [string]$Root = 'C:\Program Files\SPECviewperf 15'
)
$ErrorActionPreference = 'Continue'
function Say([string]$s) { Write-Output $s }
if ($Action -in 'launch', 'status', 'collect') {
    if ($Workload -notmatch '^[a-z][a-z_]*-[0-9]{2}$') { Say 'SPEC_GUEST REFUSED bad workload name'; exit 2 }
}
$out = 'C:\spec\out'

if ($Action -eq 'prep') {
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    New-Item -ItemType Directory -Force -Path 'C:\spec\results' | Out-Null
    $ex = 'ok'
    try {
        Add-MpPreference -ExclusionPath $Root, 'C:\spec', 'C:\kf' -ErrorAction Stop
    } catch { $ex = 'failed' }
    $free = [math]::Round((Get-PSDrive -Name C).Free / 1GB, 1)
    Say "SPEC_PREP exclusion=$ex free_gb=$free"
    exit 0
}

if ($Action -eq 'post') {
    if (-not (Test-Path (Join-Path $Root 'SPECviewperf-CLI.exe'))) { Say 'SPEC_POST cli=absent'; exit 3 }
    Say 'SPEC_POST cli=present'
    # the CLI writes viewsets\allTests.json as the standard user: without write access it fails EPERM
    $null = icacls $Root /grant 'kf:(OI)(CI)M' /T /C /Q
    Say "SPEC_POST icacls_rc=$LASTEXITCODE"
    $vc = Get-ChildItem -Path $Root -Recurse -Filter 'vcredist_x64.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($vc) {
        $p = Start-Process -FilePath $vc.FullName -ArgumentList '/install', '/quiet', '/norestart' -Wait -PassThru
        Say "SPEC_POST vcredist_rc=$($p.ExitCode)"
    } else { Say 'SPEC_POST vcredist=absent' }
    exit 0
}

if ($Action -eq 'verify') {
    $dirs = Get-ChildItem -Path $Root -Recurse -Directory -Depth 4 -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^[a-z][a-z_]*-[0-9]{2}-[0-9]+\.[0-9]+\.[0-9]+$' }
    foreach ($d in $dirs) {
        $m = Get-ChildItem -Path $d.FullName -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum
        Say ("VIEWSET {0} files={1} bytes={2}" -f $d.Name, $m.Count, [int64]$m.Sum)
    }
    Say 'SPEC_VERIFY_DONE'
    exit 0
}

if ($Action -eq 'launch') {
    $task = "kfspec_$Workload"
    try { Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue } catch { }
    $user = (Get-CimInstance Win32_ComputerSystem).UserName
    if (-not $user) { Say 'SPEC_LAUNCH fail no-interactive-user'; exit 3 }
    $dir = Join-Path $out $Workload
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    $act = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ("-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\kf\spec\spec_run.ps1 -Workload $Workload")
    $pr = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
    $set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 3) -StartWhenAvailable
    try {
        Register-ScheduledTask -TaskName $task -Action $act -Principal $pr -Settings $set -Force | Out-Null
        Start-ScheduledTask -TaskName $task
    } catch { Say "SPEC_LAUNCH fail $_"; exit 4 }
    Say "SPEC_LAUNCH ok workload=$Workload"
    exit 0
}

if ($Action -eq 'status') {
    $end = Test-Path (Join-Path (Join-Path $out $Workload) 'END.txt')
    $run = @(Get-Process -Name 'SPECviewperf-CLI' -ErrorAction SilentlyContinue).Count -gt 0
    Say ("SPEC_STATE end={0} running={1}" -f [int]$end, [int]$run)
    exit 0
}

if ($Action -eq 'collect') {
    $dir = Join-Path $out $Workload
    foreach ($f in 'cli.log', 'END.txt', 'tdr.txt') {
        Say "SPEC_COLLECT_BEGIN $f"
        $p = Join-Path $dir $f
        if (Test-Path $p) { Get-Content -Path $p -Tail 600 -ErrorAction SilentlyContinue | ForEach-Object { if ($_.Length -gt 400) { $_.Substring(0, 400) } else { $_ } } }
        Say "SPEC_COLLECT_END $f"
    }
    exit 0
}

if ($Action -eq 'kill') {
    Get-Process -Name 'SPECviewperf-CLI', 'gwpg_sysinfo' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-ScheduledTask -TaskName 'kfspec_*' -ErrorAction SilentlyContinue | ForEach-Object { Stop-ScheduledTask -TaskName $_.TaskName -ErrorAction SilentlyContinue }
    Say 'SPEC_KILL done'
    exit 0
}
