# Run only in a disposable diagnostic overlay. Downloads come from Microsoft; every binary
# is pinned and Authenticode-validated before execution. No NVIDIA registry knobs are guessed.
[CmdletBinding()]
param(
    [ValidateSet('Prepare','Status','Capture','BootEnable','BootDisable')]
    [string]$Action = 'Status',
    [ValidateRange(5,300)][int]$Seconds = 90,
    [switch]$RestartAdapter
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Root = 'C:\ProgramData\KayfabeDebugView'
$Exe = Join-Path $Root 'dbgviewcli64.exe'
$ExeSha = '7954a8bbeb1f650bb5d2b8c4c6b642fd4585319d2092433ca841c7d672a4634b'
$ZipSha = 'a8454253756af10667b82faf2323de536f0b7084d732acba63803df01ce4c316'
$FilterKey = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Debug Print Filter'
$Snapshot = Join-Path $Root 'debug-filter-original.json'
if ($RestartAdapter -and $Action -ne 'Capture') { throw '-RestartAdapter requires -Action Capture' }
if ($RestartAdapter -and $Seconds -lt 40) { throw 'Restart capture requires at least 40 seconds' }
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run from an elevated administrator shell for kernel capture.'
}
New-Item -ItemType Directory -Force -Path $Root | Out-Null

function Assert-Hash([string]$Path, [string]$Expected) {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() -ne $Expected) {
        throw "SHA256 mismatch: $Path"
    }
}
function Assert-Tool {
    Assert-Hash $Exe $ExeSha
    $sig = Get-AuthenticodeSignature -LiteralPath $Exe
    if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch '(^|, )CN=Microsoft Corporation(,|$)') {
        throw "Microsoft Authenticode verification failed: $($sig.Status)"
    }
    return [ordered]@{ sha256=$ExeSha; signature=[string]$sig.Status;
        signer=$sig.SignerCertificate.Subject; thumbprint=$sig.SignerCertificate.Thumbprint;
        version=(Get-Item -LiteralPath $Exe).VersionInfo.FileVersion }
}
# All arguments are generated here: quoting rejects embedded quotes/newlines and cannot invoke
# a shell. Paths and device IDs never end in backslash; no shell command string is executed.
function Start-Tool([string]$Program, [string[]]$Argv, [string]$Prefix) {
    foreach ($a in $Argv) {
        if ($a -match '["\r\n]' -or $a.EndsWith('\')) { throw 'Unsafe native argument shape' }
    }
    $quoted = ($Argv | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $process = Start-Process -FilePath $Program -ArgumentList $quoted -NoNewWindow -PassThru `
        -RedirectStandardOutput "$Prefix.stdout" -RedirectStandardError "$Prefix.stderr"
    $null = $process.Handle # Retain the native handle so a later refresh can retrieve exit code.
    return $process
}
function Run-Tool([string]$Program, [string[]]$Argv, [string]$Prefix, [int]$Timeout=20) {
    $p = Start-Tool $Program $Argv $Prefix
    if (-not $p.WaitForExit($Timeout * 1000)) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        throw "Native command exceeded ${Timeout}s: $Prefix"
    }
    $p.WaitForExit(); $p.Refresh()
    if ($null -eq $p.ExitCode) { throw "Native exit code unavailable; command outcome unknown: $Prefix" }
    # Read only bounded, short status/help command output here; bulk captures remain files.
    $text = Get-Content -LiteralPath "$Prefix.stdout" -Raw -ErrorAction SilentlyContinue
    return [pscustomobject]@{ exit=$p.ExitCode; stdout=[string]$text; prefix=$Prefix }
}

if ($Action -eq 'Prepare' -and -not (Test-Path -LiteralPath $Exe)) {
    $staged = Join-Path $PSScriptRoot 'dbgviewcli64.exe'
    if (Test-Path -LiteralPath $staged) {
        Assert-Hash $staged $ExeSha
        Copy-Item -LiteralPath $staged -Destination $Exe
    } else {
        $zip = Join-Path $Root 'DebugView.zip'
        Invoke-WebRequest -UseBasicParsing -TimeoutSec 60 `
            -Uri 'https://download.sysinternals.com/files/DebugView.zip' -OutFile $zip
        Assert-Hash $zip $ZipSha
        $extract = Join-Path $Root 'official-package'
        Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
        Copy-Item -LiteralPath (Join-Path $extract 'dbgviewcli64.exe') -Destination $Exe
    }
}
$tool = Assert-Tool
$run = Join-Path $Root ('run-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffffffZ'))
New-Item -ItemType Directory -Path $run | Out-Null
Write-Output "KAYFABE_DEBUGVIEW_OUTPUT=$run"
[ordered]@{ action=$Action; utc=[DateTime]::UtcNow.ToString('o'); tool=$tool;
    seconds=$Seconds; restartAdapter=[bool]$RestartAdapter } |
    ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $run 'metadata.json')

if ($Action -eq 'Prepare') {
    $version = Run-Tool $Exe @('--accepteula','--version') (Join-Path $run 'version')
    $help = Run-Tool $Exe @('--accepteula','--help') (Join-Path $run 'help')
    if ($version.exit -ne 0 -or $help.exit -ne 0) { throw 'CLI help/version failed' }
    foreach ($flag in @('--kernel','--duration','--max-lines','--log-limit','--boot-enable','--boot-disable','--status')) {
        if (-not $help.stdout.Contains($flag)) { throw "Actual executable help lacks $flag" }
    }
} elseif ($Action -eq 'Status') {
    Run-Tool $Exe @('--accepteula','--status') (Join-Path $run 'status') | ConvertTo-Json
    Run-Tool $Exe @('--accepteula','--boot-status') (Join-Path $run 'boot-status') | ConvertTo-Json
} elseif ($Action -eq 'BootEnable') {
    if (-not (Test-Path -LiteralPath $Snapshot)) {
        $key = Get-Item -LiteralPath $FilterKey -ErrorAction SilentlyContinue
        $saved = @()
        foreach ($name in @('DEFAULT','IHVVIDEO')) {
            $exists = $null -ne $key -and $key.GetValueNames() -contains $name
            $saved += [pscustomobject]@{ name=$name; existed=$exists;
                kind=$(if ($exists) { [string]$key.GetValueKind($name) } else { $null });
                value=$(if ($exists) { $key.GetValue($name) } else { $null }) }
        }
        $saved | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 $Snapshot
    }
    New-Item -Path $FilterKey -Force | Out-Null
    foreach ($name in @('DEFAULT','IHVVIDEO')) {
        New-ItemProperty -LiteralPath $FilterKey -Name $name -PropertyType DWord -Value ([uint32]::MaxValue) -Force | Out-Null
    }
    $enable = Run-Tool $Exe @('--accepteula','--boot-enable') (Join-Path $run 'boot-enable')
    if ($enable.exit -ne 0) { throw "Boot-enable failed; run BootDisable to restore filters: $run" }
    Run-Tool $Exe @('--accepteula','--boot-status') (Join-Path $run 'boot-status') | ConvertTo-Json
    Write-Output 'Boot capture armed. This script does not reboot; reboot this overlay through the bench harness.'
} elseif ($Action -eq 'BootDisable') {
    $disable = Run-Tool $Exe @('--accepteula','--boot-disable') (Join-Path $run 'boot-disable')
    if ($disable.exit -ne 0) { throw "Boot-disable failed; snapshot retained: $Snapshot" }
    if (Test-Path -LiteralPath $Snapshot) {
        foreach ($saved in @(Get-Content -LiteralPath $Snapshot -Raw | ConvertFrom-Json)) {
            if ($saved.existed) {
                New-ItemProperty -LiteralPath $FilterKey -Name $saved.name -PropertyType $saved.kind -Value $saved.value -Force | Out-Null
            } else {
                Remove-ItemProperty -LiteralPath $FilterKey -Name $saved.name -ErrorAction SilentlyContinue
            }
        }
        Move-Item -LiteralPath $Snapshot -Destination (Join-Path $run 'restored-filter-original.json')
    }
} else {
    $before = Run-Tool $Exe @('--accepteula','--status') (Join-Path $run 'status-before')
    if ($before.stdout -match '(?m)^running=true\s*$') { throw 'A DebugViewCLI capture is already running' }
    $gpu = @(Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.InstanceId -match '^PCI\\VEN_10DE&' })
    $gpu | Format-List * | Out-File -Encoding UTF8 (Join-Path $run 'gpu-before.txt')
    if ($RestartAdapter -and $gpu.Count -ne 1) { throw 'Restart requires exactly one present NVIDIA adapter' }
    $argv = @('--accepteula','--kernel','--no-win32','--verbose-kernel','--no-banner',
        '--duration',[string]$Seconds,'--max-lines','50000','--history','20000',
        '--format','csv','--log',(Join-Path $run 'kernel.csv'),'--log-limit','16','--log-wrap')
    $capture = Start-Tool $Exe $argv (Join-Path $run 'capture')
    try {
        $ready = $false
        for ($i=0; $i -lt 10; $i++) {
            $capture.Refresh()
            if ($capture.HasExited) { break }
            $status = Run-Tool $Exe @('--accepteula','--status') (Join-Path $run "status-$i") 3
            if ($status.stdout -match '(?m)^running=true\s*$') { $ready=$true; break }
            Start-Sleep -Milliseconds 200
        }
        if (-not $ready) { throw 'Capture never reported ready; adapter was not restarted' }
        if ($RestartAdapter) {
            $restart = Run-Tool "$env:SystemRoot\System32\pnputil.exe" @('/restart-device',$gpu[0].InstanceId) (Join-Path $run 'restart') 30
            $restart | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $run 'restart-result.json')
        }
        if (-not $capture.WaitForExit(($Seconds+15)*1000)) { throw 'Capture exceeded its duration bound' }
        $capture.WaitForExit(); $capture.Refresh()
        if ($null -eq $capture.ExitCode) { throw "Capture exit code unavailable; preserve $run" }
        [ordered]@{ exit=$capture.ExitCode; completedUtc=[DateTime]::UtcNow.ToString('o') } |
            ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $run 'capture-exit.json')
        if ($capture.ExitCode -ne 0) { throw "Capture exited $($capture.ExitCode); preserve $run" }
    } finally {
        $capture.Refresh()
        if (-not $capture.HasExited) {
            try { Run-Tool $Exe @('--stop') (Join-Path $run 'stop') 5 | Out-Null }
            catch { Write-Warning "Graceful capture stop failed: $_" }
            if (-not $capture.WaitForExit(5000)) { Stop-Process -Id $capture.Id -Force -ErrorAction SilentlyContinue }
        }
        Get-PnpDevice -Class Display -PresentOnly | Format-List * | Out-File -Encoding UTF8 (Join-Path $run 'gpu-after.txt')
    }
}
Write-Output "KAYFABE_DEBUGVIEW_OUTPUT=$run"
