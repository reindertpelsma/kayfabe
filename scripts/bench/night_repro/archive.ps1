# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([string]$Root,[string]$Names,[string]$DeleteHash='')
$ErrorActionPreference='Stop'
. "$PSScriptRoot\transport.ps1"
$start=(Get-Item "$Root\start.json").LastWriteTimeUtc
$items=@()
foreach($name in $Names.Split(',')) {
    if($name -notmatch '^t_[0-9]{6}\.etl$') { throw 'Invalid trace basename' }
    $path="C:\kf\hang\$name"
    if(-not (Test-Path -LiteralPath $path)) { continue }
    $f=Get-Item -LiteralPath $path
    if($f.LastWriteTimeUtc -lt $start -or $f.LastWriteTimeUtc -gt $start.AddMinutes(16)) { throw 'Trace is outside the owned capture window' }
    $sha=[Security.Cryptography.SHA256]::Create()
    $input=[IO.File]::OpenRead($path)
    try {
        $digest=[BitConverter]::ToString($sha.ComputeHash($input)).Replace('-','').ToLowerInvariant()
        if($DeleteHash) {
            if($digest -ne $DeleteHash) { throw 'Trace changed after verified transfer' }
        } else {
            $input.Position=0
            $memory=New-Object IO.MemoryStream
            $gzip=New-Object IO.Compression.GZipStream($memory,[IO.Compression.CompressionLevel]::Fastest,$true)
            $input.CopyTo($gzip); $gzip.Dispose()
            $items+=@{name=$name; sha256=$digest; gzip=[Convert]::ToBase64String($memory.ToArray()); size=[long]$f.Length}
            $memory.Dispose()
        }
    } finally { $input.Dispose(); $sha.Dispose() }
    if($DeleteHash) { Remove-Item -LiteralPath $path -Force; $items+=@{name=$name; deleted=$true; sha256=$digest} }
}
ConvertTo-N1Json @{files=$items}
