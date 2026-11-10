# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Read-only issue #305 metadata. Dot-source; no code inspection and no whole-dump read.
# Layout supplied by the owner: PAGE + DU64, u32 code at 0x38, u64 parameters at 0x40..0x58.
function ConvertFrom-LkeDumpHeader([byte[]]$Bytes) {
    if ($Bytes.Length -ne 0x1000) { throw 'SHORT_HEADER' }
    if ([Text.Encoding]::ASCII.GetString($Bytes, 0, 8) -cne 'PAGEDU64') { throw 'SIGNATURE' }
    if (-not [BitConverter]::IsLittleEndian) { throw 'ENDIAN' }
    return [ordered]@{
        bugcheck = ('0x{0:x8}' -f [BitConverter]::ToUInt32($Bytes, 0x38))
        p1 = ('0x{0:x16}' -f [BitConverter]::ToUInt64($Bytes, 0x40))
        p2 = ('0x{0:x16}' -f [BitConverter]::ToUInt64($Bytes, 0x48))
        p3 = ('0x{0:x16}' -f [BitConverter]::ToUInt64($Bytes, 0x50))
        p4 = ('0x{0:x16}' -f [BitConverter]::ToUInt64($Bytes, 0x58))
    }
}

function Read-LkeDumpHeader([string]$Path) {
    $bytes = New-Object byte[] 0x1000
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
    try {
        $read = 0
        while ($read -lt 0x1000) {
            $n = $stream.Read($bytes, $read, (0x1000 - $read))
            if ($n -eq 0) { break }
            $read += $n
        }
    } finally { $stream.Dispose() }
    if ($read -ne 0x1000) { throw 'SHORT_HEADER' }
    $row = ConvertFrom-LkeDumpHeader $bytes
    $row['bytes_read'] = $read
    return $row
}

function ConvertFrom-LkeWerReport([string]$Text) {
    $values = @{}
    foreach ($line in (($Text -replace "`0", '') -split "`r?`n")) {
        $pair = $line -split '=', 2
        if ($pair.Length -eq 2) { $values[$pair[0]] = $pair[1] }
    }
    $row = [ordered]@{ event_type = '-'; event_time = 'unknown'; event_time_raw = '-'; bucket = '-'; report_id = '-' }
    if ($values.ContainsKey('ReportIdentifier')) { $row['report_id'] = $values['ReportIdentifier'] }
    if ($values.ContainsKey('EventType')) { $row['event_type'] = $values['EventType'] }
    if ($values.ContainsKey('EventTime')) {
        $row['event_time_raw'] = $values['EventTime']
        try { $row['event_time'] = [DateTime]::FromFileTimeUtc([int64]$values['EventTime']).ToString('yyyy-MM-ddTHH:mm:ssZ') } catch { }
    }
    foreach ($key in @('Response.BucketId', 'BucketId', 'Response.LegacyBucketId')) {
        if ($values.ContainsKey($key)) { $row['bucket'] = $values[$key]; break }
    }
    # Windows names the signature fields Code, Parameter 1..4, OS version,
    # Service Pack and Product. Retain the older P1..P8 spelling as an alias.
    $labels = @('Code', 'Parameter 1', 'Parameter 2', 'Parameter 3', 'Parameter 4', 'OS version', 'Service Pack', 'Product')
    foreach ($number in 1..8) {
        $name = 'P' + $number
        $row[$name] = '-'
        foreach ($key in $values.Keys) {
            if ($key -match '^Sig\[(\d+)\]\.Name$' -and
                (@($name, $labels[$number - 1]) -icontains (($values[$key] -replace '\s+', ' ').Trim()))) {
                $valueKey = 'Sig[' + $Matches[1] + '].Value'
                if ($values.ContainsKey($valueKey)) { $row[$name] = $values[$valueKey] }
            }
        }
    }
    return $row
}

function Write-LkeWerLine([string]$Text, [string]$Report, [string]$Where) {
    $row = ConvertFrom-LkeWerReport $Text
    $fields = @('GATE_LKE', ('report=' + (ConvertTo-Json -InputObject $Report -Compress)), ('where=' + $Where))
    foreach ($key in $row.Keys) { $fields += $key + '=' + (ConvertTo-Json -InputObject ([string]$row[$key]) -Compress) }
    Write-Output ($fields -join ' ')
}

function Write-LkeDumpHeader([string]$Path) {
    $file = ConvertTo-Json -InputObject ([IO.Path]::GetFileName($Path)) -Compress
    try {
        $row = Read-LkeDumpHeader $Path
        Write-Output ('GATE_LKE_HEADER bugcheck=' + $row.bugcheck + ' p1=' + $row.p1 + ' p2=' + $row.p2 + ' p3=' + $row.p3 + ' p4=' + $row.p4 + ' bytes_read=' + $row.bytes_read + ' file=' + $file)
    } catch {
        Write-Output ('GATE_LKE_HEADER status=REFUSED reason=' + ($_.Exception.Message -replace '[^A-Za-z0-9_]', '_') + ' file=' + $file)
    }
}
