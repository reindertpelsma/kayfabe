# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# vdr_monitor_info.ps1 -- read-only (2026-10-08, VFIO DVI reference): which monitor the guest sees and how it is
# connected (WmiMonitorID / WmiMonitorConnectionParams: VideoOutputTechnology 4 = DVI, 5 = HDMI, 10 = DP external),
# the active mode, the NVIDIA adapter's state, and the HAGS registry values. Run through QGA as SYSTEM.
$ErrorActionPreference = 'Continue'
function S([uint16[]]$a) { -join ($a | Where-Object { $_ -ne 0 } | ForEach-Object { [char]$_ }) }
Get-CimInstance -Namespace root\wmi -ClassName WmiMonitorID -ErrorAction SilentlyContinue | ForEach-Object {
  'MON id ' + $_.InstanceName + ' maker=' + (S $_.ManufacturerName) + ' product=' + (S $_.ProductCodeID) + ' name=' + (S $_.UserFriendlyName) + ' year=' + $_.YearOfManufacture + ' active=' + $_.Active }
Get-CimInstance -Namespace root\wmi -ClassName WmiMonitorConnectionParams -ErrorAction SilentlyContinue | ForEach-Object {
  'MON conn ' + $_.InstanceName + ' VideoOutputTechnology=' + $_.VideoOutputTechnology }
Get-CimInstance -Namespace root\wmi -ClassName WmiMonitorListedSupportedSourceModes -ErrorAction SilentlyContinue | ForEach-Object {
  'MON modes ' + $_.InstanceName + ' n=' + $_.NumOfMonitorSourceModes + ' preferred=' + $_.PreferredMonitorSourceModeIndex }
Get-CimInstance Win32_VideoController | ForEach-Object {
  'VC ' + $_.Name + ' ' + $_.CurrentHorizontalResolution + 'x' + $_.CurrentVerticalResolution + '@' + $_.CurrentRefreshRate + ' status=' + $_.Status + ' err=' + $_.ConfigManagerErrorCode }
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
$g = Get-ItemProperty $gd
'REG HwSchMode=' + $g.HwSchMode + ' TdrDelay=' + $g.TdrDelay + ' TdrDdiDelay=' + $g.TdrDdiDelay + ' TdrLevel=' + $g.TdrLevel
'NVSMI ' + ((& 'C:\Windows\System32\nvidia-smi.exe' --query-gpu=name,driver_version,display_active,display_mode,pstate,memory.used --format=csv,noheader 2>&1) -join ' | ')
