# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# gate_state.ps1 -- read-only state lines for scripts/bench/box/windows_gate.sh, one `GATE_<ITEM> ...` line each.
#
#   -Mode desktop        the console user, dwm.exe and explorer.exe counts, the NVIDIA adapter's
#                        ConfigManagerErrorCode, the driver version, the last boot time (UTC)
#   -Mode display        only the NVIDIA adapter line (cheap; the gate polls it after the enable)
#   -Mode wer            the small WER report text (EventType, P1..P8) of LiveKernelEvent/WATCHDOG reports, the dump files' times
#                        and the display-stack events within +-120 s of each (never the dump itself)
#   -Mode tdr -Since T   the reset counters from T (an ISO-8601 UTC stamp taken IN THE GUEST): Display 4101,
#                        nvlddmkm 153, Kernel-Power 41, EventLog 6008, BugCheck 1001, WER LiveKernelEvent
#                        (code 193 counted apart, gating only with -Strict), and the live dumps written since T.
#                        Every id is printed, also with a zero (no events is the zero case, not an error).
#
# It changes nothing in the guest. `GATE_STATE_DONE mode=<m>` is the last line.
param(
    [ValidateSet('desktop', 'display', 'tdr', 'wer')][string]$Mode = 'desktop',
    [string]$Since = '',
    [switch]$Strict
)
$ErrorActionPreference = 'Continue'
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
    # The small text of the WATCHDOG / LiveKernelEvent WER reports (never the dump itself), the dump files' times
    # and every display-stack event within +-120 s of each report. Read-only.
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
        foreach ($d in Get-ChildItem $root -Directory -ErrorAction SilentlyContinue) {
            $f = Join-Path $d.FullName 'Report.wer'
            if (-not (Test-Path $f)) { continue }
            $raw = [IO.File]::ReadAllText($f)
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
    $dumps = @(Get-ChildItem C:\Windows\LiveKernelReports -Recurse -ErrorAction SilentlyContinue | Where-Object { -not $_.PSIsContainer } | Sort-Object LastWriteTime)
    foreach ($f in $dumps) { Say ("WER_DUMP " + $f.LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ssZ') + " $($f.Name) bytes=$($f.Length)") }
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
# WER LiveKernelEvents by their first parameter. 193 is a WATCHDOG live dump: the system keeps running, nothing
# resets, and it is a recorded open defect on rented-box Ada/Blackwell runs (docs/design/V3_WINDOWS_GATE.md, section 6).
# It is counted and reported separately and gates only with -Strict; every other LiveKernelEvent code gates.
$lke = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 1001; ProviderName = 'Windows Error Reporting'; StartTime = $t.ToLocalTime() } -ErrorAction SilentlyContinue |
    Where-Object { $_.Message -like '*LiveKernelEvent*' })
$lke193 = @($lke | Where-Object { $_.Message -match 'P1:\s*193\b' }).Count
$lkeOther = $lke.Count - $lke193
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
if ($Strict) { $total += $lke193 }
Say ("TDR since=$Since " + (($c.Keys | ForEach-Object { "$_=$($c[$_])" }) -join ' ') + " livekernelevent193=$lke193 strict=" + [int][bool]$Strict + " total=$total")
Get-ChildItem C:\Windows\LiveKernelReports -Recurse -ErrorAction SilentlyContinue | Where-Object { -not $_.PSIsContainer -and $_.LastWriteTimeUtc -ge $t } |
    Sort-Object LastWriteTime | ForEach-Object { Say ("LIVEDUMP " + $_.LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ssZ') + " " + $_.Name + " bytes=" + $_.Length) }
Say 'STATE_DONE mode=tdr'
exit 0
