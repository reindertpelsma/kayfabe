# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Measurement only, in our disposable native Windows lease. Never changes GPU firmware.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Setup','Collect')][string]$Action,
    [ValidateRange(1,2)][int]$Repeats = 2,
    [string]$DecodeTag = '580.82.07'
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'driver_key.ps1')
$root = 'C:\kfwc_wc1'
$build = Join-Path $root 'build'
$out = Join-Path $root 'out'
$install = Join-Path $root 'install.ps1'
New-Item -ItemType Directory -Force $out | Out-Null

function Manifest {
    $rows = @(Get-ChildItem -LiteralPath $out -File -Recurse |
        Where-Object { $_.Name -ne 'manifest.json' } | ForEach-Object {
            [ordered]@{
                path = $_.FullName.Substring($out.Length + 1).Replace('\','/')
                bytes = $_.Length
                sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLower()
            }
        })
    ConvertTo-Json -InputObject $rows -Depth 4 |
        Set-Content -Encoding ASCII -LiteralPath (Join-Path $out 'manifest.json')
}

if ($Action -eq 'Setup') {
    $names = @('kfwc.sys','kfwc_collect.exe','d3d_minimal.exe')
    $checked = @()
    foreach ($line in Get-Content -LiteralPath (Join-Path $build 'SHA256SUMS')) {
        if ($line -notmatch '^([0-9a-f]{64})  ([A-Za-z0-9_.]+)$') {
            throw 'Malformed build hash manifest.'
        }
        $expectedHash = $Matches[1]
        $fileName = $Matches[2]
        if ($fileName -notin $names -or $fileName -in $checked) {
            throw 'Unexpected or duplicate build manifest name.'
        }
        $filePath = Join-Path $build $fileName
        $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $filePath).Hash.ToLower()
        if ($actualHash -ne $expectedHash) { throw 'Uploaded executable hash mismatch.' }
        $checked += $fileName
    }
    if ($checked.Count -ne 3) { throw 'Incomplete build manifest.' }

    & $install -Action Preflight | Out-Host
    $gpu = @(Get-PnpDevice -Class Display | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' })
    if ($gpu.Count -ne 1) { throw 'Exactly one NVIDIA display device is required.' }
    try {
        if (Confirm-SecureBootUEFI) { throw 'Secure Boot is on; test signing is unavailable.' }
    } catch {
        # LTSC evaluation OVMF may have no SecureBoot variable. Other errors are not admission.
        if ($_.Exception.Message -notmatch '0xC0000100|not supported on this platform') { throw }
    }
    # Enable the documented measurement setting; native Windows defaults vary.
    # These are driver settings, not writes through the capture module.
    $key = Get-KfwcNvidiaDriverKey $gpu[0].InstanceId
    $gspChanged = $key.EnableGpuFirmware -ne 1
    New-ItemProperty -LiteralPath $key.Path -Name EnableGpuFirmware `
        -Value 1 -PropertyType DWord -Force | Out-Null
    $svc = 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
    New-ItemProperty -LiteralPath $svc -Name EnableGpuFirmware `
        -Value 1 -PropertyType DWord -Force | Out-Null
    $bcd = (& bcdedit.exe /enum '{current}') -join "`n"
    if ($LASTEXITCODE) { throw 'Cannot read BCD.' }
    if ($bcd -notmatch 'testsigning\s+Yes') {
        & $install -Action EnableTestSigning | Out-Host
        'KFWC_REBOOT_REQUIRED=1'
    } elseif ($gspChanged) { 'KFWC_REBOOT_REQUIRED=1' }
    else { 'KFWC_REBOOT_REQUIRED=0' }
    return
}

$collector = Join-Path $build 'kfwc_collect.exe'
$process = $null
$failed = $false
$ownedService = $false
try {
    if (Get-Service KayfabeWinCapture -ErrorAction SilentlyContinue) {
        throw 'A capture service already exists; it is not owned by this rehearsal.'
    }
    $ownedService = $true
    & $install -Action Install -Build $build | Out-Host
    & $collector --status
    if ($LASTEXITCODE) { throw 'Capture device status failed.' }
    $gsp = @(& nvidia-smi.exe -q | Where-Object {
        $_ -match '^\s*GSP Firmware Version\s*:'
    })
    if ($LASTEXITCODE -or $gsp.Count -ne 1 -or $gsp[0] -match ':\s*N/A\s*$') {
        throw 'Native NVIDIA GSP firmware is not active; queue discovery is not testable.'
    }
    $gsp[0].Trim() | Set-Content -Encoding ASCII (Join-Path $out 'gsp-version.txt')
    $warm = Join-Path $out 'warm'
    & $collector $warm --seconds 120 --decode-tag $DecodeTag `
        1> (Join-Path $out 'warm.stdout.txt') 2> (Join-Path $out 'warm.stderr.txt')
    if ($LASTEXITCODE) {
        throw "Warm capture failed, exit $LASTEXITCODE; preserve its sidecar."
    }
    $warmCell = Get-Content -Raw (Join-Path $warm 'cell.json') | ConvertFrom-Json
    if ($warmCell.stats.attaches -lt 1) {
        throw 'No warm binding; preserve diagnostics.'
    }
    for ($iteration = 1; $iteration -le $Repeats; $iteration++) {
        $cell = Join-Path $out "startup$iteration"
        $stop = Join-Path $out "startup$iteration.stop"
        $collectorArgs = @($cell, '--seconds', '90', '--stop-file', $stop,
            '--decode-tag', $DecodeTag)
        $process = Start-Process -FilePath $collector -ArgumentList $collectorArgs -PassThru `
            -RedirectStandardOutput (Join-Path $out "startup$iteration.stdout.txt") `
            -RedirectStandardError (Join-Path $out "startup$iteration.stderr.txt")
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $capture = Join-Path $cell 'capture.kfgspcap'
        while ((-not (Test-Path $capture)) -or (Get-Item $capture).Length -lt 128) {
            if ($process.HasExited -or $clock.Elapsed.TotalSeconds -gt 30) {
                throw 'Collector did not publish its file header.'
            }
            Start-Sleep -Milliseconds 100
        }
        $start = Get-Date
        $gpu = @(Get-PnpDevice -Class Display |
            Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' })
        if ($gpu.Count -ne 1) { throw 'GPU count changed.' }
        & pnputil.exe /restart-device $gpu[0].InstanceId | Out-Null
        $restartCode = $LASTEXITCODE
        if ($restartCode -ne 0) { throw "Device restart failed, exit $restartCode." }
        $ready = $false
        for ($sample = 0; $sample -lt 60; $sample++) {
            $state = @(Get-CimInstance Win32_VideoController |
                Where-Object { $_.PNPDeviceID -like 'PCI\VEN_10DE*' })
            if ($state.Count -eq 1 -and $state[0].ConfigManagerErrorCode -eq 0) {
                $ready = $true
                break
            }
            Start-Sleep -Seconds 1
        }
        if (-not $ready) { throw 'NVIDIA device did not return to Code 0.' }
        & (Join-Path $build 'd3d_minimal.exe') |
            Set-Content -Encoding ASCII (Join-Path $out "d3d$iteration.json")
        if ($LASTEXITCODE) { throw "Hardware D3D readback failed, exit $LASTEXITCODE." }
        if (-not $process.WaitForExit(120000)) { throw 'Owned collector did not finish.' }
        if ($process.ExitCode) { throw "Startup collector failed, exit $($process.ExitCode)." }
        $events = @()
        try {
            $events = @(Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$start})
        } catch {
            if ($_.FullyQualifiedErrorId -notmatch 'NoMatchingEventsFound') { throw }
        }
        $summary = [ordered]@{
            iteration = $iteration
            restart_exit = $restartCode
            code = [int]$state[0].ConfigManagerErrorCode
            system_1001 = @($events | Where-Object Id -eq 1001).Count
            system_6008 = @($events | Where-Object Id -eq 6008).Count
            system_4101 = @($events | Where-Object Id -eq 4101).Count
            display_errors = @($events | Where-Object {
                $_.ProviderName -in @('nvlddmkm','Display') -and $_.Level -le 2
            }).Count
        }
        $summary | ConvertTo-Json | Set-Content -Encoding ASCII `
            (Join-Path $out "health$iteration.json")
        if ($summary.system_1001 + $summary.system_6008 + $summary.system_4101 +
            $summary.display_errors -ne 0) { throw 'Native display/system error during capture.' }
        $process = $null
    }
} catch {
    $failed = $true
    # Avoid serializing arbitrary exception objects or machine identity.
    $_.Exception.Message | Set-Content -Encoding ASCII (Join-Path $out 'failure.txt')
} finally {
    if ($process -and -not $process.HasExited) {
        New-Item -ItemType File -Force (Join-Path $out "startup$iteration.stop") | Out-Null
        if (-not $process.WaitForExit(10000)) { Stop-Process -Id $process.Id -Force }
    }
    try {
        if ($ownedService) {
            & $install -Action Remove 1> (Join-Path $out 'remove.txt') 2>&1
        }
    } finally { Manifest }
}
if ($failed) { exit 1 }
'KFWC_COLLECT_COMPLETE'
