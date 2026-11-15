# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# licence_probe.ps1 -- READ-ONLY readout of the Windows evaluation-licence state (slmgr /dlv and /xpr, the wlms service, the clock).
# STATUS: CODE, 2026-11-12 (claude/spec-pc-20261112); changes nothing in the guest.
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
Write-Output 'LIC_DONE'
