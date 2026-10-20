# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([string]$Root,[string]$DeleteHash='')
$ErrorActionPreference='Stop'
. "$PSScriptRoot\transport.ps1"
if($Root -notmatch '^C:\\kf\\n1\\run[0-9]+$') { throw 'Not an owned run root' }
$m=Get-Content "$Root\start.json" -Raw | ConvertFrom-Json
Stop-ScheduledTask kf-pmmon
if((Get-ScheduledTask kf-pmmon).State -eq 'Running') { throw 'Monitor still running; no prune permitted' }
$path='C:\kf\hang\pm_live.csv'
$f=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite)
try {
    if($f.Length -lt [long]$m.pm_offset) { throw 'PM prefix changed; no prune permitted' }
    $null=$f.Seek([long]$m.pm_offset,[IO.SeekOrigin]::Begin)
    $memory=New-Object IO.MemoryStream
    $f.CopyTo($memory); $bytes=$memory.ToArray(); $memory.Dispose()
    $sha=[Security.Cryptography.SHA256]::Create()
    try { $hash=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','').ToLowerInvariant() } finally { $sha.Dispose() }
    if($DeleteHash) {
        if($hash -ne $DeleteHash) { throw 'PM tail changed; no prune permitted' }
        if($f.Length -ne [long]$m.pm_offset+$bytes.Length) { throw 'Concurrent PM writer; no prune permitted' }
        $f.SetLength([long]$m.pm_offset); $f.Flush($true)
        ConvertTo-N1Json @{truncated=$true; retained_prefix_bytes=[long]$m.pm_offset; sha256=$hash}
    } else {
        $mem=New-Object IO.MemoryStream
        $gz=New-Object IO.Compression.GZipStream($mem,[IO.Compression.CompressionLevel]::Fastest,$true)
        $gz.Write($bytes,0,$bytes.Length); $gz.Dispose()
        ConvertTo-N1Json @{name='pm_tail.bin'; sha256=$hash; gzip=[Convert]::ToBase64String($mem.ToArray()); size=$bytes.Length; retained_prefix_bytes=[long]$m.pm_offset}
        $mem.Dispose()
    }
} finally { $f.Dispose() }
