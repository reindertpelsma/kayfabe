# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Export data logs only. Stop/drain the collector before running this script.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][string]$OutputPath,
    [ValidateRange(1,1024)][int]$MaxInputMiB = 64,
    [ValidateRange(1,1000000)][int]$MaxRecords = 100000
)
$ErrorActionPreference = 'Stop'
$Path = [IO.Path]::GetFullPath($Path)
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
if (Test-Path -LiteralPath $OutputPath) { throw 'Output already exists; choose a fresh text-log path.' }
$temporary = $OutputPath + '.tmp.' + [Guid]::NewGuid().ToString('N')
$stream = $null; $reader = $null; $writer = $null; $output = $null; $hasher = $null
$published = $false
function Read-Exact([int]$Count) {
    $bytes = $reader.ReadBytes($Count)
    if ($bytes.Length -ne $Count) { throw 'Truncated capture; stop/drain the collector before export.' }
    return ,$bytes
}
function U32([byte[]]$Bytes, [int]$Offset) { return [BitConverter]::ToUInt32($Bytes,$Offset) }
function U64([byte[]]$Bytes, [int]$Offset) { return [BitConverter]::ToUInt64($Bytes,$Offset) }
function Write-Line($Value) { $writer.WriteLine(($Value | ConvertTo-Json -Depth 6 -Compress)) }
try {
    # Sharing permits other readers but excludes a live writer. No mutation of input.
    $stream = [IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    if ($stream.Length -lt 64 -or $stream.Length -gt ([int64]$MaxInputMiB * 1048576)) { throw 'Capture is empty, truncated, or exceeds MaxInputMiB.' }
    $reader = [IO.BinaryReader]::new($stream)
    $header = Read-Exact 64
    if ((U32 $header 0) -ne 0x5457474b -or (U32 $header 4) -ne 1 -or (U32 $header 8) -ne 64 -or (U32 $header 12) -ne 64 -or (U32 $header 32) -ne 1 -or (U64 $header 16) -eq 0 -or (U32 $header 36) -ne 0 -or (U64 $header 40) -ne 0 -or (U64 $header 48) -ne 0 -or (U64 $header 56) -ne 0) { throw 'Unsupported capture header.' }
    $output = [IO.File]::Open($temporary,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    $writer = [IO.StreamWriter]::new($output,[Text.UTF8Encoding]::new($false))
    Write-Line ([ordered]@{schema='kayfabe-gsp-text/1';kind='header';source_name=[IO.Path]::GetFileName($Path);magic=(U32 $header 0);version=1;header_bytes=64;record_header_bytes=64;qpc_frequency=(U64 $header 16);started_qpc=(U64 $header 24);flags=1;reserved=0;reserved2=@(0,0,0);capture_complete=$false})
    $count = 0
    while ($stream.Position -lt $stream.Length) {
        if ($count -ge $MaxRecords) { throw 'Capture exceeds MaxRecords; no complete export produced.' }
        $record = Read-Exact 64
        $size = U32 $record 8
        $flags = U32 $record 48
        if ((U32 $record 0) -ne 0x5247474b -or (U32 $record 4) -ne 64 -or $size -lt 80 -or $size -gt 65536 -or $size % 8 -ne 0 -or (U32 $record 12) -gt 1 -or ($flags -band 1) -eq 0 -or ($flags -band 0xfffffff8L) -ne 0 -or (U32 $record 60) -ne 0) { throw 'Invalid capture record framing.' }
        $payload = Read-Exact ([int]$size)
        Write-Line ([ordered]@{kind='record';magic=(U32 $record 0);header_bytes=64;payload_bytes=$size;direction=(U32 $record 12);qpc=(U64 $record 16);table_pa=(U64 $record 24);queue_sequence=(U32 $record 32);rpc_sequence=(U32 $record 36);rpc_function=(U32 $record 40);rpc_result=(U32 $record 44);flags=$flags;missing_before=(U32 $record 52);rpc_version=(U32 $record 56);reserved=0;payload_hex=([BitConverter]::ToString($payload)).Replace('-','').ToLowerInvariant()})
        $count++
        if ($count % 256 -eq 0) { $writer.Flush() }
    }
    $hasher = [Security.Cryptography.SHA256]::Create()
    $stream.Position = 0
    $digest = ([BitConverter]::ToString($hasher.ComputeHash($stream))).Replace('-','').ToLowerInvariant()
    $stats = $null
    $statsPath = $Path + '.stats.json'
    if (Test-Path -LiteralPath $statsPath) {
        if ((Get-Item -LiteralPath $statsPath).Length -gt 65536) { throw 'Statistics sidecar exceeds 64 KiB.' }
        $stats = [IO.File]::ReadAllText($statsPath) | ConvertFrom-Json
    }
    Write-Line ([ordered]@{kind='footer';records=$count;source_bytes=$stream.Length;source_sha256=$digest;file_export_complete=$true;capture_complete=$false;driver_stats=$stats})
    $writer.Flush(); $output.Flush($true); $writer.Dispose(); $writer=$null; $output=$null
    [IO.File]::Move($temporary,$OutputPath)
    $published=$true
    [ordered]@{text_log=$OutputPath;records=$count;source_bytes=$stream.Length;source_sha256=$digest;capture_complete=$false} | ConvertTo-Json -Compress
} finally {
    if ($writer) { $writer.Dispose() }
    if ($output) { $output.Dispose() }
    if ($reader) { $reader.Dispose() }
    if ($stream) { $stream.Dispose() }
    if ($hasher) { $hasher.Dispose() }
    if (-not $published -and (Test-Path -LiteralPath $temporary)) { Remove-Item -LiteralPath $temporary -Force }
}
