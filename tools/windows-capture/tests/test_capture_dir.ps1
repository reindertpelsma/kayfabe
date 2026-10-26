# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([Parameter(Mandatory)][string]$Helper)
$ErrorActionPreference = 'Stop'
. $Helper
$root = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory $root | Out-Null
$failed = 0
try {
    foreach ($name in 'warm','startup1') {
        try {
            $path = Join-Path $root $name
            $actual = New-KfwcCaptureDirectory $path
            if ($actual -ne $path -or -not (Test-Path -LiteralPath $path -PathType Container)) {
                throw 'Missing capture directory.'
            }
            if (@(Get-ChildItem -LiteralPath $path -Force).Count) {
                throw 'Directory is not empty.'
            }
            "ok creates_$name"
        } catch { "FAIL creates_$name"; $failed++ }
    }
    $existing = Join-Path $root 'occupied'
    New-Item -ItemType Directory $existing | Out-Null
    Set-Content (Join-Path $existing 'prior.txt') 'preserve'
    try {
        $null = New-KfwcCaptureDirectory $existing
        throw 'unexpected acceptance'
    } catch {
        if ($_.Exception.Message -eq 'unexpected acceptance') {
            'FAIL rejects_existing'; $failed++
        } else { 'ok rejects_existing' }
    }
    if ((Get-Content (Join-Path $existing 'prior.txt')) -ne 'preserve') {
        'FAIL preserves_existing'; $failed++
    } else { 'ok preserves_existing' }
} finally { Remove-Item -LiteralPath $root -Recurse -Force }
if ($failed) { exit 1 }
