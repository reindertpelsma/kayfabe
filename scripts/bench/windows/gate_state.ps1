# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# gate_state.ps1 -- read-only state lines for scripts/bench/box/windows_gate.sh, one `GATE_<ITEM> ...` line each.
#
#   -Mode desktop        the console user, dwm.exe and explorer.exe counts, the NVIDIA adapter's
#                        ConfigManagerErrorCode, the driver version, the last boot time (UTC)
#   -Mode display        only the NVIDIA adapter line (cheap; the gate polls it after the enable)
#   -Mode tdr -Since T   the reset counters from T (an ISO-8601 UTC stamp taken IN THE GUEST): Display 4101,
#                        nvlddmkm 153, Kernel-Power 41, EventLog 6008, BugCheck 1001, WER LiveKernelEvent
#                        (code 193 counted apart, gating only with -Strict), and the live dumps written since T.
#                        Every id is printed, also with a zero (no events is the zero case, not an error).
#
# It changes nothing in the guest. `GATE_STATE_DONE mode=<m>` is the last line.
param(
    [ValidateSet('desktop', 'display', 'tdr')][string]$Mode = 'desktop',
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
