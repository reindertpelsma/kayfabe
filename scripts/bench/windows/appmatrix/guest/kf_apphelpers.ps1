# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_apphelpers.ps1 -- small helpers the generated per-app scripts (C:\kf\apps\ID.ps1) dot-source.
# They print the app's own output to stdout (the supervisor captures it) and leave the exit code in
# $global:KfRc, so an app script ends with `exit $global:KfRc`.
$global:KfRc = 0

# first file called $Name under $Root (recursive), or $null
function Find-KfFile {
    param([string]$Root, [string]$Name)
    $f = Get-ChildItem -Path $Root -Recurse -Filter $Name -File -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($f) { return $f.FullName }
    return $null
}

# run a console program in the foreground; its stdout+stderr go to our stdout, exit code to $global:KfRc
function Invoke-KfExe {
    param([string]$Exe, [string[]]$ArgList = @(), [string]$Cwd = '')
    if ($Cwd) { Push-Location $Cwd }
    try { & $Exe @ArgList 2>&1 | ForEach-Object { "$_" }; $global:KfRc = $LASTEXITCODE }
    catch { "KFERR $_"; $global:KfRc = 127 }
    finally { if ($Cwd) { Pop-Location } }
}

# run a (usually windowed) program for $Seconds and require that it is still alive at the end:
# an early exit is the failure (crash, device lost), staying up under load is the success.
function Invoke-KfSurvive {
    param([string]$Exe, [string[]]$ArgList = @(), [int]$Seconds = 30, [string]$Cwd = '', [string]$Tag = 'app')
    $sp = @{ FilePath = $Exe; PassThru = $true }
    if ($ArgList.Count -gt 0) { $sp.ArgumentList = $ArgList }
    if ($Cwd) { $sp.WorkingDirectory = $Cwd }
    try { $p = Start-Process @sp } catch { "KFSURVIVE $Tag START_FAILED $_"; $global:KfRc = 127; return }
    $null = $p.Handle
    $i = 0
    while ($i -lt $Seconds) { Start-Sleep -Seconds 1; $i++; if ($p.HasExited) { break } }
    if ($p.HasExited) {
        "KFSURVIVE $Tag EXITED_EARLY rc=$($p.ExitCode) after=$i s"
        $global:KfRc = $(if ($p.ExitCode -ne 0) { $p.ExitCode } else { 1 })
        return
    }
    "KFSURVIVE $Tag ALIVE_AT $Seconds s"
    & taskkill.exe /PID $p.Id /T /F 2>&1 | Out-Null
    $global:KfRc = 0
}

# run a program that is expected to finish by itself within $TimeoutS (benchmarks with -close 1 etc.)
function Invoke-KfWait {
    param([string]$Exe, [string[]]$ArgList = @(), [int]$TimeoutS = 120, [string]$Cwd = '', [string]$Tag = 'app')
    $sp = @{ FilePath = $Exe; PassThru = $true }
    if ($ArgList.Count -gt 0) { $sp.ArgumentList = $ArgList }
    if ($Cwd) { $sp.WorkingDirectory = $Cwd }
    try { $p = Start-Process @sp } catch { "KFWAIT $Tag START_FAILED $_"; $global:KfRc = 127; return }
    $null = $p.Handle
    if ($p.WaitForExit($TimeoutS * 1000)) { "KFWAIT $Tag EXITED rc=$($p.ExitCode)"; $global:KfRc = $p.ExitCode }
    else { "KFWAIT $Tag STILL_RUNNING_AFTER $TimeoutS s"; & taskkill.exe /PID $p.Id /T /F 2>&1 | Out-Null; $global:KfRc = 124 }
}

# sha256 (first 16 hex) of a string, for the host-vs-guest output digests (OUTSHA lines)
function Get-KfDigest {
    param([string]$Text)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $h = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($Text))
    return (($h | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 16)
}
