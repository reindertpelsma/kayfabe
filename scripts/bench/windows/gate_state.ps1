# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# gate_state.ps1 -- read-only state lines for scripts/bench/box/windows_gate.sh, one `GATE_<ITEM> ...` line each.
#
#   -Mode desktop        the console user, dwm.exe and explorer.exe counts, the NVIDIA adapter's
#                        ConfigManagerErrorCode, the driver version, the last boot time (UTC)
#   -Mode display        only the NVIDIA adapter line (cheap; the gate polls it after the enable)
#   -Mode wer            every Report.wer decoded, the LiveKernelEvent/WATCHDOG timeline, dump file times and
#                        display-stack events within +-120 s; only 0x1000 bytes of each recursively found dump
#   -Mode tdr -Since T   the reset counters from T (an ISO-8601 UTC stamp taken IN THE GUEST): Display 4101,
#                        nvlddmkm 153, Kernel-Power 41, EventLog 6008, BugCheck 1001, WER LiveKernelEvent
#                        (distinct 193 reports and raw entries apart, gating only with -Strict), and live dumps since T.
#                        Every id is printed, also with a zero (no events is the zero case, not an error).
#
# It changes nothing in the guest. `GATE_STATE_DONE mode=<m>` is the last line.
param(
    [ValidateSet('desktop', 'display', 'tdr', 'wer')][string]$Mode = 'desktop',
    [string]$Since = '',
    [switch]$Strict
)
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lke_decode.ps1')
function Say([string]$s) { Write-Output ("GATE_" + (($s -replace '\s+', ' ').Trim())) }

function Show-Display {
    $d = Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | Where-Object { $_.PNPDeviceID -like 'PCI\VEN_10DE*' }
    if (-not $d) {
        $p = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' } | Select-Object -First 1
        if ($p) { Say ("DISPLAY name='$($p.FriendlyName)' driver=none code=$($p.Problem) status=$($p.Status)") }
        else { Say 'DISPLAY none' }
        return
    }
    $p = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -eq $d.PNPDeviceID } | Select-Object -First 1
    Say ("DISPLAY name='$($d.Name)' driver=$($d.DriverVersion) code=$($d.ConfigManagerErrorCode) status=$($d.Status) pnp=$($p.Status)")
}

if ($Mode -eq 'display') { Show-Display; Say 'STATE_DONE mode=display'; exit 0 }

if ($Mode -eq 'desktop') {
    $os = Get-CimInstance Win32_OperatingSystem
    Say ("BOOT utc=" + $os.LastBootUpTime.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') + " os='$($os.Caption)' build=$($os.BuildNumber)")
    $user = (Get-CimInstance Win32_ComputerSystem).UserName
    $dwm = @(Get-Process -Name dwm -ErrorAction SilentlyContinue).Count
    $exp = @(Get-Process -Name explorer -ErrorAction SilentlyContinue).Count
    $sess = (query user 2>&1 | Out-String) -replace '\s+', ' '
    Say "DESKTOP user='$user' dwm=$dwm explorer=$exp"
    Say "SESSIONS $sess"
    Show-Display
    Say 'STATE_DONE mode=desktop'
    exit 0
}

if ($Mode -eq 'wer') {
    # Decode every small Report.wer; retain the existing WATCHDOG / LiveKernelEvent timeline and nearby events.
    # Dump contents are limited to a validated 0x1000-byte header read in the guest. Read-only.
    #   GATE_WER_REPORT dir=<name> where=<archive|queue> mtime=<utc> event_time=<utc from the report's FILETIME>
    #   GATE_WER_PARAM <dir> <key>=<value>        EventType, Sig[n] (P1..P8), DynamicSig[1..2]
    #   GATE_WER_DUMP <utc> <name> bytes=<n>
    #   GATE_WER_NEAR report=<dir> dt=<+-seconds> <utc> <provider> <id>
    $wer = 'C:\ProgramData\Microsoft\Windows\WER'
    $keep = '^(EventType|ReportType|ReportStatus|FriendlyEventName|Sig\[\d+\]\.(Name|Value)|DynamicSig\[(1|2)\]\.(Name|Value))='
    $times = @()
    foreach ($where in 'ReportArchive', 'ReportQueue') {
        $root = Join-Path $wer $where
        if (-not (Test-Path $root)) { continue }
        foreach ($report in Get-ChildItem $root -Filter 'Report.wer' -File -Recurse -ErrorAction SilentlyContinue) {
            $f = $report.FullName
            $d = $report.Directory
            try {
                $raw = [IO.File]::ReadAllText($f)
                Write-LkeWerLine $raw $d.Name $where
            } catch {
                Write-Output ('GATE_LKE status=REFUSED reason=REPORT_IO report=' + (ConvertTo-Json -InputObject $d.Name -Compress))
                continue
            }
            if ($raw -replace "`0", '' -notmatch 'LiveKernelEvent|WATCHDOG|Kernel_1') { continue }
            $lines = ($raw -replace "`0", '') -split "`r?`n"
            $et = ($lines | Where-Object { $_ -match '^EventTime=' } | Select-Object -First 1) -replace '^EventTime=', ''
            $evUtc = ''
            if ($et -match '^\d+$') { $evUtc = [DateTime]::FromFileTimeUtc([int64]$et).ToString('yyyy-MM-ddTHH:mm:ssZ') }
            $mt = $d.LastWriteTimeUtc
            $times += @{ dir = $d.Name; t = $(if ($evUtc) { [DateTime]::Parse($evUtc).ToUniversalTime() } else { $mt }) }
            Say ("WER_REPORT dir=$($d.Name) where=$where mtime=" + $mt.ToString('yyyy-MM-ddTHH:mm:ssZ') + " event_time=$evUtc")
            foreach ($l in $lines) { if ($l -match $keep) { Say ("WER_PARAM $($d.Name) $l") } }
        }
    }
    $dumpRoots = @('C:\Windows\LiveKernelReports', (Join-Path $wer 'ReportArchive'), (Join-Path $wer 'ReportQueue'))
    $dumps = @(foreach ($root in $dumpRoots) {
        if (Test-Path $root) { Get-ChildItem -LiteralPath $root -Filter '*.dmp' -File -Recurse -ErrorAction SilentlyContinue }
    }) | Sort-Object FullName -Unique
    $dumps = @($dumps)
    foreach ($f in $dumps) { Say ("WER_DUMP " + $f.LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ssZ') + " $($f.Name) bytes=$($f.Length)") }
    # Only the first 0x1000 bytes, read in the guest. Never transfer the dump.
    $headers = @($dumps)
    foreach ($header in $headers) { Write-LkeDumpHeader $header.FullName }
    Write-Output ('GATE_LKE_HEADERS files=' + $headers.Count)
    $anchors = @($times)
    foreach ($f in $dumps) { $anchors += @{ dir = $f.Name; t = $f.LastWriteTimeUtc } }
    foreach ($a in $anchors) {
        $from = $a.t.AddSeconds(-120).ToLocalTime()
        $to = $a.t.AddSeconds(120).ToLocalTime()
        $ev = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $from; EndTime = $to } -ErrorAction SilentlyContinue |
            Where-Object { $_.ProviderName -match 'nvlddmkm|Display|dxgkrnl|Dxgkrnl|Kernel-Power|BugCheck' })
        foreach ($e in $ev) {
            $dt = [int]($e.TimeCreated.ToUniversalTime() - $a.t).TotalSeconds
            Say ("WER_NEAR report=$($a.dir) dt=$dt " + $e.TimeCreated.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') + " $($e.ProviderName) $($e.Id)")
        }
        if (-not $ev) { Say ("WER_NEAR report=$($a.dir) none within +-120s") }
    }
    Say 'STATE_DONE mode=wer'
    exit 0
}

# tdr
if (-not $Since) { Say 'REFUSED tdr needs -Since'; exit 2 }
$t = [DateTime]::Parse($Since, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal)
function CountEv([string]$log, [string]$prov, [int]$id, [string]$like = '') {
    $f = @{ LogName = $log; Id = $id; StartTime = $t.ToLocalTime() }
    if ($prov) { $f['ProviderName'] = $prov }
    $e = @(Get-WinEvent -FilterHashtable $f -ErrorAction SilentlyContinue)
    if ($like) { $e = @($e | Where-Object { $_.Message -like $like }) }
    return $e.Count
}
# Count distinct 193 reports, not WER's repeated event-1001 submissions.
# A report identity includes its own EventTime, never the entry's TimeCreated.
$lkeAll = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 1001; ProviderName = 'Windows Error Reporting'; StartTime = $t.ToLocalTime() } -ErrorAction SilentlyContinue |
    Where-Object { $_.Message -like '*LiveKernelEvent*' })
$reports = @()
foreach ($where in 'ReportArchive', 'ReportQueue') {
    $root = Join-Path 'C:\ProgramData\Microsoft\Windows\WER' $where
    if (-not (Test-Path $root)) { continue }
    foreach ($f in Get-ChildItem $root -Filter 'Report.wer' -File -Recurse -ErrorAction SilentlyContinue) {
        try { $raw = [IO.File]::ReadAllText($f.FullName) } catch { continue }
        $lines = ($raw -replace "`0", '') -split "`r?`n"
        $et = ($lines | Where-Object { $_ -match '^EventTime=\d+$' } | Select-Object -First 1) -replace '^EventTime=', ''
        $evt = $null
        try { if ($et) { $evt = [DateTime]::FromFileTimeUtc([int64]$et) } } catch { }
        $rid = ($lines | Where-Object { $_ -match '^ReportIdentifier=' } | Select-Object -First 1) -replace '^ReportIdentifier=', ''
        $rid = if ($rid) { $rid.Trim().ToLowerInvariant() } else { '' }
        $decoded = ConvertFrom-LkeWerReport $raw
        $signature = @('P1','P2','P3','P4','P5' | ForEach-Object { [string]$decoded[$_] })
        $signature = if ($signature -contains '-') { '' } else { ($signature -join '|').ToLowerInvariant() }
        $identity = if ($rid) { 'id:' + $rid } else { 'dir:' + $f.Directory.Name.ToLowerInvariant() }
        $ownTime = if ($null -ne $evt) { $evt.ToFileTimeUtc().ToString() } else { '?' }
        $reports += [pscustomobject]@{ key=($identity + '|time:' + $ownTime); time=$evt; id=$rid;
            directory=$f.Directory.Name.ToLowerInvariant(); store=$where; signature=$signature }
    }
}
function Event-Signature($e) {
    $parts = @()
    foreach ($number in 1..5) {
        $label = if ($number -eq 1) { '(?:P1|Code)' } else { '(?:P' + $number + '|Parameter\s+' + ($number-1) + ')' }
        if ($e.Message -notmatch ('(?:^|\s)' + $label + ':\s*(\S+)')) { return '' }
        $parts += $Matches[1].ToLowerInvariant()
    }
    return $parts -join '|'
}
function Find-Report($e) {
    $id = ''; $directory = ''; $store = ''
    if ($e.Message -match 'Report Id:\s*([0-9a-fA-F-]{36})') { $id = $Matches[1].ToLowerInvariant() }
    if ($e.Message -match '\\WER\\Report(Archive|Queue)\\([^\\\r\n]+)') {
        $directory = $Matches[2].Trim().ToLowerInvariant(); $store = 'Report' + $Matches[1]
    }
    $found = @()
    if ($directory) { $found = @($reports | Where-Object { $_.directory -eq $directory -and $_.store -eq $store }) }
    if (-not $found -and $id) { $found = @($reports | Where-Object { $_.id -eq $id }) }
    $signature = Event-Signature $e
    if (-not $found -and $signature -and -not $id -and -not $directory -and (Is-193 $e)) { $found = @($reports | Where-Object { $_.signature -eq $signature }) }
    # Archive/queue copies of the same identity/EventTime are one report. A
    # signature may resolve an existing report only when the match is unique.
    $unique = @($found | Group-Object key | ForEach-Object { $_.Group[0] })
    if ($unique.Count -gt 1 -and $signature) { $unique = @($unique | Where-Object { $_.signature -eq $signature }) }
    if ($unique.Count -eq 1) { return $unique[0] }
    if ($found.Count -eq 0) {
        $identity = if ($id) { 'id:' + $id } elseif ($directory) { 'dir:' + $directory } else { $null }
        return [pscustomobject]@{ key=$(if ($identity) { $identity + '|time:?' } else { $null }); time=$null }
    }
    return [pscustomobject]@{ key=$null; time=$null }
}
function Is-193($e) { return $e.Message -match '(?:^|\s)(?:P1|Code):\s*(?:0x)?193\b' }
$lke = @()
$lkeStale = 0
$lke193Reports = @{}
$lkeUnresolved = 0
foreach ($e in $lkeAll) {
    $report = Find-Report $e
    $rt = $report.time
    if ($null -ne $rt -and $rt -lt $t) {
        $lkeStale++
        Say ("LKE_STALE event=" + $e.TimeCreated.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') + " report_event_time=" + $rt.ToString('yyyy-MM-ddTHH:mm:ssZ'))
    } else {
        $lke += $e
        if (Is-193 $e) {
            $key = $report.key
            if ($null -eq $rt -or -not $report.key) { $lkeUnresolved++ }
            # Missing identity is not a report. Keep those entries visible and
            # make the comparison UNKNOWN rather than inventing distinct IDs.
            if ($key) { $lke193Reports[$key] = $true }
        }
    }
}
$lke193Entries = @($lke | Where-Object { Is-193 $_ }).Count
$lke193 = $lke193Reports.Count
# Other codes remain entry counts; 193 resubmissions never become "other" events.
$lkeOther = $lke.Count - $lke193Entries
$c = [ordered]@{
    display4101      = CountEv 'System' 'Display' 4101
    nvlddmkm153      = CountEv 'System' 'nvlddmkm' 153
    kernelpower41    = CountEv 'System' 'Microsoft-Windows-Kernel-Power' 41
    eventlog6008     = CountEv 'System' 'EventLog' 6008
    bugcheck1001     = CountEv 'System' 'Microsoft-Windows-WER-SystemErrorReporting' 1001
    livekernelevent_other = $lkeOther
}
$total = 0
foreach ($k in $c.Keys) { $total += [int]$c[$k] }
if ($Strict) { $total += $lke193Entries }
Say ("TDR since=$Since " + (($c.Keys | ForEach-Object { "$_=$($c[$_])" }) -join ' ') + " livekernelevent193=$lke193 livekernelevent193_entries=$lke193Entries livekernelevent193_unresolved=$lkeUnresolved livekernelevent_stale=$lkeStale strict=" + [int][bool]$Strict + " total=$total")
Get-ChildItem C:\Windows\LiveKernelReports -Recurse -ErrorAction SilentlyContinue | Where-Object { -not $_.PSIsContainer -and $_.LastWriteTimeUtc -ge $t } |
    Sort-Object LastWriteTime | ForEach-Object { Say ("LIVEDUMP " + $_.LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ssZ') + " " + $_.Name + " bytes=" + $_.Length) }
Say 'STATE_DONE mode=tdr'
exit 0
