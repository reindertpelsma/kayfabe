# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# The `windows-guest-setup` job of .github/workflows/ci.yml: scripts/windows/kayfabe-windows-setup.* against the REAL registry of a
# disposable GitHub-hosted Windows runner (Windows PowerShell 5.1, elevated), with a faked adapter class key.
#
# STATUS: LIVE, 2026-11-20. FALSIFIER (written before the run): a value is missing, has the wrong type or the wrong number after phase
# pre or post; the second run writes again; an unelevated run (a fresh standard local user) exits with anything but 2 or changes the
# registry; `auto` picks post on a machine with no NVIDIA device; `post` with no device or a hostile key does not refuse. Everything the
# script creates is removed in `finally`, and the keys the runner already had are put back.
# What this cannot prove: a real NVIDIA device (the runner has none), a driver install, a reboot. Those are the hardware check.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$src = Join-Path $repo 'scripts\windows'
$bat = Join-Path $src 'kayfabe-windows-setup.bat'
$ps1 = Join-Path $src 'kayfabe-windows-setup.ps1'
$env:KF_SETUP_NOPAUSE = '1'

$class = '{4d36e968-e325-11ce-bfc1-08002be10318}'
$fakeKey = "$class\0099"
$classPath = "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$fakeKey"
$svcPath = 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
$dwmPath = 'HKLM:\SOFTWARE\Microsoft\Windows\Dwm'
$failures = New-Object System.Collections.Generic.List[string]

function Check([bool]$Condition, [string]$Message) {
    if ($Condition) { Write-Host "PASS  $Message" } else { Write-Host "FAIL  $Message"; $failures.Add($Message) }
}
function Get-Dword([string]$Path, [string]$Name) {
    $p = Get-ItemProperty -LiteralPath $Path -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $p) { return $null }
    return $p.$Name
}
function Get-Kind([string]$Path, [string]$Name) {
    try { return (Get-Item -LiteralPath $Path).GetValueKind($Name).ToString() } catch { return '' }
}
function Invoke-Bat([string[]]$Arguments) {
    $out = & $bat @Arguments 2>&1 | ForEach-Object { "$_" }
    $rc = $LASTEXITCODE
    $out | ForEach-Object { Write-Host "    | $_" }
    return @{ rc = $rc; text = ($out -join "`n") }
}

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'the job must run elevated (GitHub-hosted Windows runners do); without it the checks below prove nothing'
}
Write-Host "PowerShell $($PSVersionTable.PSVersion) on $([Environment]::OSVersion.VersionString)"

# What the runner had before, to put back.
$svcExisted = Test-Path -LiteralPath $svcPath
$svcHad = Get-Dword $svcPath 'EnableGpuFirmware'
$dwmHad = Get-Dword $dwmPath 'OverlayTestMode'
$classHad = Test-Path -LiteralPath $classPath
$user = 'kfsetupunpriv'
$userMade = $false
$stage = 'C:\Users\Public\kfsetup-check'
$plain = $null

try {
    if ($classHad) { throw "the fake class key $classPath already exists" }
    New-Item -Path $classPath -Force | Out-Null

    # 1. Unelevated: a new standard local user runs the script; it must refuse (exit 2) and change nothing.
    $plain = 'Kf!' + [guid]::NewGuid().ToString('N') + 'aZ9'
    $secure = ConvertTo-SecureString $plain -AsPlainText -Force
    New-LocalUser -Name $user -Password $secure -AccountNeverExpires -PasswordNeverExpires | Out-Null
    $userMade = $true
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item -LiteralPath $ps1 -Destination $stage
    & icacls.exe $stage /grant 'Users:(OI)(CI)RX' | Out-Null
    foreach ($phase in @('pre', 'post', 'auto')) {
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = 'powershell.exe'
        $psi.Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$stage\kayfabe-windows-setup.ps1`" -Phase $phase"
        $psi.UserName = $user
        $psi.Domain = $env:COMPUTERNAME
        $psi.Password = $secure
        $psi.LoadUserProfile = $true
        $psi.UseShellExecute = $false
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $psi.WorkingDirectory = 'C:\Windows\Temp'
        $proc = [System.Diagnostics.Process]::Start($psi)
        $text = $proc.StandardOutput.ReadToEnd() + $proc.StandardError.ReadToEnd()
        $proc.WaitForExit()
        $text -split "`r?`n" | ForEach-Object { if ($_) { Write-Host "    | $_" } }
        Check ($proc.ExitCode -eq 2) "unelevated -Phase $phase exits 2 (got $($proc.ExitCode))"
        Check ($text -match 'elevated PowerShell' -and $text -match 'Nothing was changed') "unelevated -Phase $phase says it needs an elevated PowerShell and changed nothing"
        Check ((Test-Path -LiteralPath $svcPath) -eq $svcExisted -and $null -eq (Get-Dword $classPath 'EnableGpuFirmware')) "unelevated -Phase $phase left the registry as it was"
    }

    # 2. phase pre through the .bat: both pre values, DWORD, read back; the missing service key is created.
    $r = Invoke-Bat @('-Phase', 'pre')
    Check ($r.rc -eq 0) "pre exits 0 (got $($r.rc))"
    Check ((Get-Dword $svcPath 'EnableGpuFirmware') -eq 1 -and (Get-Kind $svcPath 'EnableGpuFirmware') -eq 'DWord') 'pre: nvlddmkm EnableGpuFirmware is DWORD 1'
    Check ((Get-Dword $dwmPath 'OverlayTestMode') -eq 5 -and (Get-Kind $dwmPath 'OverlayTestMode') -eq 'DWord') 'pre: Dwm OverlayTestMode is DWORD 5'
    Check ($null -eq (Get-Dword $classPath 'EnableGpuFirmware')) 'pre: the class key is not touched'
    Check ($r.text -match 'readback HKLM\\SYSTEM\\CurrentControlSet\\Services\\nvlddmkm EnableGpuFirmware=1 OK') 'pre prints the read-back of the service key'
    Check ($r.text -match 'readback HKLM\\SOFTWARE\\Microsoft\\Windows\\Dwm OverlayTestMode=5 OK') 'pre prints the read-back of the Dwm value'

    # 3. idempotent: the second run says "already" and the state is the same.
    $r = Invoke-Bat @('-Phase', 'pre')
    Check ($r.rc -eq 0) 'second pre exits 0'
    Check ($r.text -match 'already  HKLM\\SYSTEM\\CurrentControlSet\\Services\\nvlddmkm' -and $r.text -notmatch 'set      HKLM') 'second pre writes nothing (already ...)'
    Check ((Get-Dword $svcPath 'EnableGpuFirmware') -eq 1 -and (Get-Dword $dwmPath 'OverlayTestMode') -eq 5) 'second pre: same values'

    # 4. auto on a machine with no NVIDIA device: pre. post with no device: refused (3), nothing written to the fake key.
    $r = Invoke-Bat @()
    Check ($r.rc -eq 0 -and $r.text -match 'auto -> pre') 'auto with no NVIDIA device runs pre'
    $r = Invoke-Bat @('-Phase', 'post')
    Check ($r.rc -eq 3 -and $r.text -match 'REFUSED') "post with no NVIDIA device is refused with exit 3 (got $($r.rc))"
    Check ($null -eq (Get-Dword $classPath 'EnableGpuFirmware')) 'refused post wrote no class value'

    # 5. post with the adapter's key given (the faked class key 0099): the class value, a restart notice, idempotent.
    $r = Invoke-Bat @('-Phase', 'post', '-DriverKey', $fakeKey)
    Check ($r.rc -eq 0) "post with a given key exits 0 (got $($r.rc))"
    Check ((Get-Dword $classPath 'EnableGpuFirmware') -eq 1 -and (Get-Kind $classPath 'EnableGpuFirmware') -eq 'DWord') 'post: class key 0099 EnableGpuFirmware is DWORD 1'
    Check ($r.text -match 'RESTART REQUIRED') 'post says a restart is required'
    $r = Invoke-Bat @('-Phase', 'post', '-DriverKey', $fakeKey)
    Check ($r.rc -eq 0 -and $r.text -match 'already  HKLM\\SYSTEM\\CurrentControlSet\\Control\\Class' -and $r.text -notmatch 'set      HKLM') 'second post writes nothing'
    $r = Invoke-Bat @('-Phase', 'post', '-DriverKey', "$class\..\..\Run")
    Check ($r.rc -eq 2) "post with a key that leaves the class key is refused with exit 2 (got $($r.rc))"

    # 6. a wrong value that is already there is corrected (and the correction is read back).
    New-ItemProperty -LiteralPath $svcPath -Name EnableGpuFirmware -PropertyType DWord -Value 0 -Force | Out-Null
    $r = Invoke-Bat @('-Phase', 'pre')
    Check ($r.rc -eq 0 -and (Get-Dword $svcPath 'EnableGpuFirmware') -eq 1 -and $r.text -match '\(was 0\)') 'a wrong existing value is corrected and reported'

    # Nothing sensitive in the output of any of the runs above.
    Check (-not ($r.text -match [regex]::Escape($env:COMPUTERNAME)) -and -not ($r.text -match [regex]::Escape($env:USERNAME))) 'the output names neither the computer nor the user'
}
finally {
    Write-Host 'cleanup'
    try {
        if (Test-Path -LiteralPath $classPath) { Remove-Item -LiteralPath $classPath -Recurse -Force }
        if (-not $svcExisted) {
            if (Test-Path -LiteralPath $svcPath) { Remove-Item -LiteralPath $svcPath -Recurse -Force }
        } elseif ($null -eq $svcHad) {
            Remove-ItemProperty -LiteralPath $svcPath -Name EnableGpuFirmware -ErrorAction SilentlyContinue
        } else {
            New-ItemProperty -LiteralPath $svcPath -Name EnableGpuFirmware -PropertyType DWord -Value $svcHad -Force | Out-Null
        }
        if ($null -eq $dwmHad) {
            Remove-ItemProperty -LiteralPath $dwmPath -Name OverlayTestMode -ErrorAction SilentlyContinue
        } else {
            New-ItemProperty -LiteralPath $dwmPath -Name OverlayTestMode -PropertyType DWord -Value $dwmHad -Force | Out-Null
        }
        if ($userMade) { Remove-LocalUser -Name $user -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
        Check (-not (Test-Path -LiteralPath $classPath)) 'cleanup: the fake class key is gone'
        Check ((Test-Path -LiteralPath $svcPath) -eq $svcExisted) 'cleanup: the service key is as the runner had it'
        Check ((Get-Dword $dwmPath 'OverlayTestMode') -eq $dwmHad) 'cleanup: the Dwm value is as the runner had it'
    } catch {
        Write-Host "cleanup error: $($_.Exception.Message)"
        $failures.Add('cleanup failed')
    }
}

if ($failures.Count -gt 0) {
    Write-Host "`n$($failures.Count) check(s) failed:"
    $failures | ForEach-Object { Write-Host " - $_" }
    exit 1
}
Write-Host "`nall checks passed"
exit 0
