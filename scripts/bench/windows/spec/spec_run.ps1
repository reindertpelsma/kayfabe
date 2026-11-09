# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# spec_run.ps1 -Workload NAME -- ONE CLI run of one viewset (1 iteration, FullHD), started by
# spec_guest.ps1 -Action launch in the console user's INTERACTIVE session. Writes C:\spec\out\<Workload>\
# {start.txt, cli.log, cli.err, tdr.txt, display.txt, END.txt}; END.txt is written last
# (`SPECRUN_END workload=<w> rc=<n> start=<utc> end=<utc> secs=<n>`).
#
# The CLI's own result directory (C:\spec\results\<Workload>) holds the scores; this script never reads it
# and nothing copies it out. The reset counters come from C:\kf\gate_state.ps1 (the Windows gate staged it
# in this image) with the instant this run started, taken in the guest.
#
# STATUS: CODE, 2026-11-07, NOT RUN on Windows (docs/design/V3_SPEC_GATE.md). Modelled on the
# 2026-11-07 native reference kit (traces/specviewperf_native_20261107/kit/spec_run.ps1).
param(
    [Parameter(Mandatory = $true)][string]$Workload,
    [string]$Out = 'C:\spec\out',
    [string]$Root = 'C:\Program Files\SPECviewperf 15'
)
$ErrorActionPreference = 'Continue'
if ($Workload -notmatch '^[a-z][a-z_]*-[0-9]{2}$') { exit 2 }
$d = Join-Path $Out $Workload
New-Item -ItemType Directory -Force $d | Out-Null
Remove-Item (Join-Path $d '*') -Force -ErrorAction SilentlyContinue
$t0 = (Get-Date).ToUniversalTime()
$stamp0 = $t0.ToString('yyyy-MM-ddTHH:mm:ssZ')
"SPECRUN_START workload=$Workload utc=$stamp0" | Set-Content (Join-Path $d 'start.txt')
$res = "C:\spec\results\$Workload"
New-Item -ItemType Directory -Force $res | Out-Null
$cli = Join-Path $Root 'SPECviewperf-CLI.exe'
Set-Location $Root
$p = Start-Process -FilePath $cli -ArgumentList '--workload', $Workload, '--iterations', '1', '--resolution', 'FullHD', '--resultDirectory', $res `
    -WorkingDirectory $Root -RedirectStandardOutput (Join-Path $d 'cli.log') -RedirectStandardError (Join-Path $d 'cli.err') -PassThru -WindowStyle Hidden
$null = $p.Handle   # keep the handle so ExitCode survives WaitForExit
$p.WaitForExit()
$rc = $p.ExitCode
$t1 = (Get-Date).ToUniversalTime()
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\gate_state.ps1 -Mode tdr -Since $stamp0 *> (Join-Path $d 'tdr.txt')
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\gate_state.ps1 -Mode display *> (Join-Path $d 'display.txt')
"SPECRUN_END workload=$Workload rc=$rc start=$stamp0 end=$($t1.ToString('yyyy-MM-ddTHH:mm:ssZ')) secs=$([int]($t1 - $t0).TotalSeconds)" | Set-Content (Join-Path $d 'END.txt')
