# Observation only. Run solely inside the inherited, separately granted BW6 arm.
param([Parameter(Mandatory=$true)][double]$ReadinessSeconds)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
if($ReadinessSeconds -le 0){throw 'Inherited readiness deadline required'}
$root='C:\kf'
$evidence=Join-Path $root 'bw8_observation.jsonl'
if(Test-Path $evidence){throw 'Existing observation: never replay enable'}
$gpu=@(Get-PnpDevice -PresentOnly -Class Display |
    Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($gpu.Count -ne 1 -or [int]$gpu[0].Problem -ne 22){throw 'Require one disabled GPU'}
$driver=(Get-PnpDeviceProperty -InstanceId $gpu[0].InstanceId `
    -KeyName DEVPKEY_Device_DriverVersion).Data
if($driver -ne '32.0.15.8088'){throw 'Expected580.88'}
$clock=[Diagnostics.Stopwatch]::StartNew()
$beginUtc=[DateTime]::UtcNow
$first=$null
$cursors=@{}
foreach($channel in 'System','Application'){
    $latest=Get-WinEvent -LogName $channel -MaxEvents 1 -ErrorAction Stop
    $cursors[$channel]=[long]$latest.RecordId
}

function Events($filter){
    try{return @(Get-WinEvent -FilterHashtable $filter -ErrorAction Stop)}
    catch{
        if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*'){throw}
        return @()
    }
}

function DumpMetadata {
    # Read WER text and filesystem names only; never open a dump or command buffer.
    $rows=@{}
    $names=@{}
    $werRoots=@('C:\ProgramData\Microsoft\Windows\WER\ReportArchive',
        'C:\ProgramData\Microsoft\Windows\WER\ReportQueue')
    foreach($dir in $werRoots){
        if(!(Test-Path -LiteralPath $dir)){continue}
        $reports=@(Get-ChildItem -LiteralPath $dir -Recurse -Filter Report.wer)
        if($reports.Count -gt 4096){throw 'WER inventory overflow'}
        foreach($report in $reports){
            if($report.Length -gt 1048576){throw 'WER text overflow'}
            $text=Get-Content -LiteralPath $report.FullName -Raw
            $code=''
            if($text -match '(?m)^Sig\[0\]\.Value=(?:0x)?([0-9a-fA-F]+)\s*$'){
                $code='0x'+$matches[1].ToLower()
            }
            if($text -notmatch '(?m)^EventType=LiveKernelEvent\s*$' -and
                $text -notmatch '(?m)^EventType=BlueScreen\s*$'){continue}
            $reportId=$report.FullName+'|'+$report.Length+'|'+$report.LastWriteTimeUtc.Ticks
            $rows[$reportId]=@{id=$reportId;code=$code}
            $fields=@{}
            foreach($line in $text -split '\r?\n'){
                if($line -match '^(File\[\d+\]\.(?:Original\.Path|Path))=(.+)$'){
                    $fields[$matches[1]]=$matches[2]
                }
            }
            foreach($key in @($fields.Keys)){
                if($key -notmatch '^File\[(\d+)\]\.Original\.Path$'){continue}
                $copyKey='File['+$matches[1]+'].Path'
                $original=$fields[$key]
                if($original.StartsWith('\\?\')){$original=$original.Substring(4)}
                if(!$original.EndsWith('.dmp',[StringComparison]::OrdinalIgnoreCase)){continue}
                if(!$fields.ContainsKey($copyKey)){continue}
                $copy=Join-Path $report.DirectoryName $fields[$copyKey]
                if(!(Test-Path -LiteralPath $copy -PathType Leaf)){continue}
                $item=Get-Item -LiteralPath $copy
                $names[$original]=@($names[$original])+@(@{code=$code;length=$item.Length;
                    ticks=$item.LastWriteTimeUtc.Ticks})
            }
        }
    }
    foreach($dir in 'C:\Windows\LiveKernelReports','C:\Windows\Minidump'){
        if(!(Test-Path -LiteralPath $dir)){continue}
        foreach($dump in @(Get-ChildItem -LiteralPath $dir -Recurse -Filter *.dmp)){
            $code=''
            $candidates=@($names[$dump.FullName] | Where-Object {
                $_.length -eq $dump.Length -and $_.ticks -eq $dump.LastWriteTimeUtc.Ticks
            } | ForEach-Object {$_.code} | Select-Object -Unique)
            if($candidates.Count -eq 1){$code=$candidates[0]}
            $dumpId=$dump.FullName+'|'+$dump.Length+'|'+$dump.LastWriteTimeUtc.Ticks
            $rows[$dumpId]=@{id=$dumpId;code=$code}
        }
    }
    if(Test-Path -LiteralPath 'C:\Windows\MEMORY.DMP'){
        $rows['memory']=@{id='MEMORY.DMP';code=''}
    }
    if($rows.Count -gt 4096){throw 'Dump inventory overflow'}
    return @($rows.Values)
}

function Emit($kind){
    $events=@()
    $system=Events @{LogName='System';Id=1001,41,6008,4101;StartTime=$beginUtc}
    $application=Events @{LogName='Application';Id=1001;StartTime=$beginUtc;
        ProviderName='Windows Error Reporting'}
    foreach($channel in 'System','Application'){
        $latest=Get-WinEvent -LogName $channel -MaxEvents 1 -ErrorAction Stop
        if([long]$latest.RecordId -lt $cursors[$channel]){throw 'Event log cursor reset'}
    }
    foreach($event in @($system)+@($application)){
        if([long]$event.RecordId -le $cursors[$event.LogName]){continue}
        $events+=@{channel=$event.LogName;provider=$event.ProviderName;
            id=[int]$event.Id;record_id=[long]$event.RecordId}
    }
    if($events.Count -gt 4096){throw 'Event inventory overflow'}
    $g=@(Get-PnpDevice -PresentOnly -Class Display |
        Where-Object InstanceId -like 'PCI\VEN_10DE*')
    if($g.Count -ne 1 -or $null -eq $g[0].Problem){throw 'Expected one adapter/code'}
    $version=(Get-PnpDeviceProperty -InstanceId $g[0].InstanceId `
        -KeyName DEVPKEY_Device_DriverVersion).Data
    $boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
    $row=[ordered]@{kind=$kind;boot=$boot.ToUniversalTime().ToString('o');
        code=[int]$g[0].Problem;driver=$version;
        events=@($events);dumps=@(DumpMetadata);complete=$true}
    $row['utc']=[DateTime]::UtcNow.ToString('o')
    $row['mono']=$clock.Elapsed.TotalSeconds
    if($kind -eq 'start'){$row['cursors']=$cursors}
    $line=$row|ConvertTo-Json -Depth 6 -Compress
    Add-Content -LiteralPath $evidence -Value $line -Encoding UTF8
    Write-Output $line
    $script:lastCode=$row['code']
    $script:lastMono=$row['mono']
    return
}

Emit 'start'
Emit 'enable'
# The original single-enable helper retains its disabled/driver/once-marker guards.
$child=Start-Process powershell.exe -PassThru -WindowStyle Hidden -ArgumentList @(
    '-NoProfile','-ExecutionPolicy','Bypass','-File',"$root\enable580_once.ps1") `
    -RedirectStandardOutput "$root\bw8_enable_stdout.txt" `
    -RedirectStandardError "$root\bw8_enable_stderr.txt"
$enable=$clock.Elapsed.TotalSeconds
while($true){
    if($null -ne $first -and $clock.Elapsed.TotalSeconds-$first -ge 60){
        Emit 'end'
        break
    }
    Emit 'sample'
    if($null -eq $first -and $lastCode -eq 0){$first=$lastMono}
    if($null -eq $first -and $clock.Elapsed.TotalSeconds-$enable -gt $ReadinessSeconds){
        throw 'NoCode0 within inherited readiness deadline'
    }
    Start-Sleep -Seconds 2
}
if(!$child.HasExited){throw 'Single-enable helper still running'}
if($child.ExitCode -ne 0){throw 'Single-enable helper failed'}
