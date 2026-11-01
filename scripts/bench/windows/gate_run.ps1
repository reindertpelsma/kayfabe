# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# gate_run.ps1 -- run ONE probe executable in the guest with a hard time limit, for scripts/bench/box/windows_gate.sh.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\gate_run.ps1 -Exe C:\kf\probes\cup8.exe [-Arg d3d11] [-TimeoutS 120]
#
# Prints the probe's own output (stdout and stderr, merged) and, last, `GATE_PROBE exe=<name> arg=<a> rc=<n> secs=<s> timeout=<0|1>`.
# A probe that does not end in -TimeoutS seconds is killed and reported timeout=1 rc=-1: a hung GPU call must not hang the gate.
# Changes nothing in the guest except the two temp files under C:\kf\probes.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Arg = '',
    [int]$TimeoutS = 120
)
$ErrorActionPreference = 'Continue'
$name = [IO.Path]::GetFileName($Exe)
if ($Exe -notmatch '^C:\\kf\\probes\\[A-Za-z0-9_.-]+\.exe$') { Write-Output "GATE_PROBE exe=$name REFUSED not under C:\kf\probes"; exit 2 }
if (-not (Test-Path $Exe)) { Write-Output "GATE_PROBE exe=$name REFUSED missing"; exit 2 }
$out = "C:\kf\probes\$name.out.txt"
$err = "C:\kf\probes\$name.err.txt"
Remove-Item $out, $err -ErrorAction SilentlyContinue
$sw = [Diagnostics.Stopwatch]::StartNew()
$a = @{ FilePath = $Exe; RedirectStandardOutput = $out; RedirectStandardError = $err; PassThru = $true; WindowStyle = 'Hidden' }
if ($Arg) { $a['ArgumentList'] = $Arg }
$p = Start-Process @a
$done = $p.WaitForExit($TimeoutS * 1000)
$to = 0
$rc = -1
if (-not $done) {
    $to = 1
    try { $p.Kill() } catch {}
} else {
    $rc = $p.ExitCode
}
$sw.Stop()
Get-Content $out -ErrorAction SilentlyContinue
Get-Content $err -ErrorAction SilentlyContinue
Write-Output ("GATE_PROBE exe=$name arg=$Arg rc=$rc secs=" + [int]$sw.Elapsed.TotalSeconds + " timeout=$to")
exit 0
