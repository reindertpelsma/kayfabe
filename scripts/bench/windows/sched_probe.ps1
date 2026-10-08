# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sched_probe.ps1 -- read-only (2026-10-08): the guest's GPU scheduling mode and user-mode-submission related state.
# One line per fact: `SCH <what> <value>`.
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
"SCH build " + (Get-CimInstance Win32_OperatingSystem).BuildNumber
$p = Get-ItemProperty $gd -ErrorAction SilentlyContinue
foreach ($n in 'HwSchMode', 'HwSchModeApplicationOverride', 'TdrLevel', 'TdrDelay', 'DisableUserModeSubmission', 'EnableUserModeSubmission', 'UserModeSubmission') {
  "SCH reg GraphicsDrivers\$n = " + $(if ($null -eq $p.$n) { 'absent' } else { $p.$n })
}
Get-ChildItem $gd -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match 'Scheduler|Feature|UMS|Submission' } | ForEach-Object {
  $k = $_; "SCH key " + $k.PSChildName + " :: " + (($k.Property | ForEach-Object { "$_=" + $k.GetValue($_) }) -join '; ')
}
$cls = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
Get-ChildItem $cls -ErrorAction SilentlyContinue | Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DriverDesc -like 'NVIDIA*' } | ForEach-Object {
  $q = Get-ItemProperty $_.PSPath
  foreach ($n in 'HwSchMode', 'FeatureControl', 'EnableMsHybrid', 'RMInstLoc') { "SCH nvkey " + $_.PSChildName + " $n = " + $(if ($null -eq $q.$n) { 'absent' } else { $q.$n }) }
}
Get-PnpDevice -Class Display | ForEach-Object { "SCH adapter '$($_.FriendlyName)' $($_.Status) $($_.Problem)" }
