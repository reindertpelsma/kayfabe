# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# hwsch_off.ps1 — guest-side VARIABLE (2026-10-08, not yet run): HwSchMode = 1 (hardware-accelerated GPU
# scheduling OFF) for the next boot. Run through QGA as SYSTEM. Nothing outside the guest is touched.
$gd = 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
Set-ItemProperty -Path $gd -Name HwSchMode -Type DWord -Value 1
'HWSCH HwSchMode=' + (Get-ItemProperty $gd).HwSchMode
