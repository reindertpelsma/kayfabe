# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# nvidia_device.ps1 -- 2026-10-08: enable or disable the guest's NVIDIA display device (pnputil), for the Basic
# Display fallback desktop of windows_broker.sh (the NVIDIA driver disabled, Windows draws on kf3's GOP framebuffer).
# The action is the first line of this file as run: set $Action below by prepending `$Action='disable'` or 'enable'.
if (-not $Action) { $Action = 'status' }
$d = Get-PnpDevice -Class Display | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.Present }
$d | ForEach-Object { "NV before: $($_.FriendlyName) Status=$($_.Status) Problem=$($_.Problem) $($_.InstanceId)" }
if ($Action -in 'disable', 'enable') {
  $d | ForEach-Object { pnputil "/$Action-device" "$($_.InstanceId)" 2>&1 | ForEach-Object { "pnputil: $_" } }
}
Get-PnpDevice -Class Display | ForEach-Object { "after: $($_.FriendlyName) Status=$($_.Status) Problem=$($_.Problem) Present=$($_.Present)" }
