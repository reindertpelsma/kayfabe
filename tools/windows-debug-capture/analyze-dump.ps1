# Microsoft SDK KD, pinned on the controller. Analyze a local dump; never attach to a process.
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$DumpPath,
    [ValidateRange(30,600)][int]$Seconds = 240
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Root = 'C:\ProgramData\KayfabeKD'
$ZipHash = 'a3883456c4631bb46b9091a2d3197f17984df99ba89c36c2f8e54c976052d800'
$ManifestHash = '4d1cf80283e34641a1e057b1596a5ff6eca25b9992e247bba0798b29c0ec2958'
function Assert-Hash([string]$Path, [string]$Hash) {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() -ne $Hash) {
        throw "SHA256 mismatch: $Path"
    }
}
$dump = Get-Item -LiteralPath $DumpPath
if ($dump.PSIsContainer -or $dump.Length -gt 2GB -or
    $dump.FullName -notmatch '^C:\\Windows\\LiveKernelReports\\WATCHDOG\\[^\\]+\.dmp$') {
    throw 'Expected one local WATCHDOG dump of at most 2 GiB'
}
New-Item -ItemType Directory -Force -Path $Root | Out-Null
$run = Join-Path $Root ('run-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffffffZ'))
New-Item -ItemType Directory -Path $run | Out-Null
Write-Output "KAYFABE_KD_OUTPUT=$run"
$zip = Join-Path $PSScriptRoot 'kf-kd-bundle.zip'
Assert-Hash $zip $ZipHash
$tools = Join-Path $run 'tools'
Expand-Archive -LiteralPath $zip -DestinationPath $tools
$manifest = Join-Path $tools 'manifest.json'
Assert-Hash $manifest $ManifestHash
$signatures = @()
foreach ($file in (Get-Content -Raw -LiteralPath $manifest | ConvertFrom-Json)) {
    if ($file.path -notmatch '^x64/[A-Za-z0-9_./-]+$' -or $file.path.Contains('..')) {
        throw 'Unexpected manifest path'
    }
    $path = Join-Path $tools $file.path
    Assert-Hash $path $file.sha256
    if ([IO.Path]::GetExtension($path) -in @('.dll','.exe')) {
        $signature = Get-AuthenticodeSignature -LiteralPath $path
        if ($signature.Status -ne 'Valid' -or
            $signature.SignerCertificate.Subject -notmatch '(^|, )CN=Microsoft Corporation(,|$)') {
            throw "Microsoft Authenticode validation failed: $($file.path): $($signature.Status)"
        }
        $signatures += [ordered]@{ path=$file.path; sha256=$file.sha256;
            signature=[string]$signature.Status; signer=$signature.SignerCertificate.Subject }
    }
}
$bin = Join-Path $tools 'x64'
$cache = Join-Path $Root 'symbols'
New-Item -ItemType Directory -Force -Path $cache | Out-Null
# Separate script lines prevent semicolons being consumed as symbol-path separators.
# Explicit extension loading avoids relying on a default extension chain.
$commands = @('.symfix C:\ProgramData\KayfabeKD\symbols', '.sympath',
    '.reload /f nt', '.reload /f dxgkrnl.sys', '.reload /f watchdog.sys',
    ".load $bin\winext\ext.dll", '.chain', '.bugcheck', '!ext.analyze -v',
    'lmvm nvlddmkm', 'kv', '.enumtag', '.echo KAYFABE_KD_COMMANDS_FINISHED', 'q')
$commandFile = Join-Path $run 'commands.txt'
$commands | Set-Content -Encoding ASCII -LiteralPath $commandFile
$argv = @('-noshell', '-sins', '-y', 'srv*C:\ProgramData\KayfabeKD\symbols*https://msdl.microsoft.com/download/symbols',
    '-z', $dump.FullName, '-cf', $commandFile)
foreach ($arg in $argv) {
    if ($arg -match '["\r\n]' -or $arg.EndsWith('\')) { throw 'Unsafe native argument shape' }
}
$arguments = ($argv | ForEach-Object { '"' + $_ + '"' }) -join ' '
$stdout = Join-Path $run 'analysis.stdout'
$stderr = Join-Path $run 'analysis.stderr'
[ordered]@{ utc=[DateTime]::UtcNow.ToString('o'); dump=$dump.FullName;
    dumpSize=$dump.Length; dumpSha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $dump.FullName).Hash;
    seconds=$Seconds; commands=$commands; signatures=$signatures } |
    ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $run 'metadata.json')
$oldExtensions = $env:_NT_DEBUGGER_EXTENSION_PATH
$oldAltSymbols = $env:_NT_ALT_SYMBOL_PATH
$env:_NT_DEBUGGER_EXTENSION_PATH = "$bin\winext;$bin\winxp;$bin"
$env:_NT_ALT_SYMBOL_PATH = $null
$process = $null
$reason = $null
try {
    $process = Start-Process -FilePath (Join-Path $bin 'kd.exe') -ArgumentList $arguments `
        -WorkingDirectory $bin -NoNewWindow -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $null = $process.Handle
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (-not $process.WaitForExit(250)) {
        if ($watch.Elapsed.TotalSeconds -ge $Seconds) { $reason='timeout'; break }
        $total = 0L
        foreach ($file in @($stdout,$stderr)) {
            if (Test-Path -LiteralPath $file) { $total += (Get-Item -LiteralPath $file).Length }
        }
        if ($total -gt 32MB) { $reason='output exceeded 32 MiB'; break }
    }
    if ($reason) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
    $process.WaitForExit(); $process.Refresh()
    if ($null -eq $process.ExitCode) { throw 'Debugger exit code unavailable' }
    $analysisComplete = $false
    if (-not $reason -and (Get-Item -LiteralPath $stdout).Length -le 32MB) {
        $output = Get-Content -Raw -LiteralPath $stdout
        $analysisComplete = ($output -match 'Bugcheck Analysis' -and
            $output -match 'BUGCHECK_CODE:' -and $output -match 'STACK_TEXT:' -and
            $output -match 'KAYFABE_KD_COMMANDS_FINISHED' -and
            $output -notmatch 'No export analyze found|Unable to add extension DLL|is not extension gallery command')
    }
    [ordered]@{ exit=$process.ExitCode; stoppedBecause=$reason;
        elapsedSeconds=$watch.Elapsed.TotalSeconds; processComplete=($null -eq $reason);
        analysisComplete=$analysisComplete } |
        ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $run 'result.json')
    if ($reason -or $process.ExitCode -ne 0 -or -not $analysisComplete) {
        throw "KD analysis incomplete: $reason, exit $($process.ExitCode), semantic check=$analysisComplete"
    }
} finally {
    if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
    $env:_NT_DEBUGGER_EXTENSION_PATH = $oldExtensions
    $env:_NT_ALT_SYMBOL_PATH = $oldAltSymbols
}
Write-Output "KAYFABE_KD_OUTPUT=$run"
