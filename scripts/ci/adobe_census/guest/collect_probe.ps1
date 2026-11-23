# Pass an installed app executable and, where available, its sibling sniffer.
param(
    [Parameter(Mandatory=$true)][string]$AppExe,
    [Parameter(Mandatory=$true)][string]$Out,
    [string]$Sniffer,
    [switch]$Photoshop
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Path $Out | Out-Null
$version = [Diagnostics.FileVersionInfo]::GetVersionInfo($AppExe).ProductVersion
if ($version -notmatch '^\d+(\.\d+){1,4}') { $version = 'unknown' }
else { $version = $Matches[0] }
"version`t$version" | Set-Content -Encoding ascii -LiteralPath (Join-Path $Out 'version.tsv')

if ($Sniffer) {
    $lines = & $Sniffer 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Adobe sniffer failed' }
    $pattern = '^(CUDA|OpenCL) system device count:\s*\d+\s*$'
    if ($Photoshop) { $pattern = '^(Start|End) platform (OpenCL|native)\s*$' }
    @($lines | Where-Object { $_ -match $pattern }) |
        Set-Content -Encoding ascii -LiteralPath (Join-Path $Out 'sniffer.txt')
}
