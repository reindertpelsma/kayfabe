# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# licence_probe.ps1 -- READ-ONLY readout of the Windows evaluation-licence state (slmgr /dlv and /xpr, the wlms service, the clock).
# STATUS: CODE, 2026-11-12 (claude/spec-pc-20261112); 2026-11-17: the System log's event 1074 (LIC_EVT1074) was added. Changes nothing
# in the guest. specviewperf_stage.sh keeps this raw readout private and prints only what scripts/bench/specviewperf/licence.py
# extracts from it (state text; no key, no computer name, no account).
$utc = (Get-Date).ToUniversalTime().ToString('o')
Write-Output "LIC_TIME utc=$utc local=$((Get-Date).ToString('o')) tz=$((Get-TimeZone).Id)"
$os = Get-CimInstance Win32_OperatingSystem
Write-Output ("LIC_OS caption='{0}' build={1} install={2} lastboot={3}" -f $os.Caption, $os.BuildNumber, $os.InstallDate.ToUniversalTime().ToString('o'), $os.LastBootUpTime.ToUniversalTime().ToString('o'))
Get-Service | Where-Object { $_.Name -match 'wlms|sppsvc|W32Time' } | ForEach-Object { Write-Output ("LIC_SERVICE {0} status={1} start={2}" -f $_.Name, $_.Status, $_.StartType) }
Get-Process -Name wlms -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ("LIC_PROC wlms pid={0} start={1}" -f $_.Id, $_.StartTime.ToUniversalTime().ToString('o')) }
Write-Output "LIC_XPR_BEGIN"
& cscript.exe //nologo "$env:SystemRoot\System32\slmgr.vbs" /xpr 2>&1 | ForEach-Object { Write-Output "  $_" }
Write-Output "LIC_DLV_BEGIN"
& cscript.exe //nologo "$env:SystemRoot\System32\slmgr.vbs" /dlv 2>&1 | ForEach-Object { Write-Output "  $_" }
Write-Output "LIC_W32TM_BEGIN"
& w32tm.exe /query /status 2>&1 | ForEach-Object { Write-Output "  $_" }
try {
    $k = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\wlms' -ErrorAction Stop
    Write-Output ("LIC_REG wlms ImagePath='{0}' Start={1}" -f $k.ImagePath, $k.Start)
} catch { Write-Output 'LIC_REG wlms absent' }
# Event 1074 of the System log: a process initiated a shutdown or restart. Properties of the event: 0 process, 1 computer, 2 reason
# text, 3 reason code, 4 shutdown type, 5 comment, 6 user. The computer (1) and the user (6) are never written.
function Get-Clean($v) { return (([string]$v) -replace "[\r\n']", ' ') }
try {
    Get-WinEvent -FilterHashtable @{ LogName = 'System'; Id = 1074 } -MaxEvents 8 -ErrorAction Stop | ForEach-Object {
        $p = $_.Properties
        Write-Output ("LIC_EVT1074 time={0} process='{1}' reason='{2}' code={3} type='{4}' comment='{5}'" -f $_.TimeCreated.ToUniversalTime().ToString('o'), (Get-Clean $p[0].Value), (Get-Clean $p[2].Value), (Get-Clean $p[3].Value), (Get-Clean $p[4].Value), (Get-Clean $p[5].Value))
    }
} catch { Write-Output 'LIC_EVT1074_NONE' }
Write-Output 'LIC_DONE'
