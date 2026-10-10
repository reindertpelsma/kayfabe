# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# rm_instloc.ps1 -- 2026-10-08 (run62): set the guest RM's `RMInstLoc` registry DWORD (`nvrm_registry.h`: USERD = bits 17:16,
# VID = 3, so 0x30000 = "USERD in local video memory", every other field DEFAULT) in the NVIDIA adapter's driver key and in the
# nvlddmkm service keys, and print what is there. Takes effect when the driver next starts (reboot). Prepend `$Value = <dword>`
# to choose another value; `$Value = -1` removes it.
if ($null -eq $Value) { $Value = 0x30000 }
$cls = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
$keys = @(Get-ChildItem $cls -ErrorAction SilentlyContinue | Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DriverDesc -like 'NVIDIA*' } | ForEach-Object { $_.PSPath })
$keys += 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm', 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm\Parameters'
foreach ($k in $keys) {
  if (-not (Test-Path $k)) { New-Item -Path $k -Force | Out-Null }
  if ($Value -eq -1) { Remove-ItemProperty -Path $k -Name RMInstLoc -ErrorAction SilentlyContinue }
  else { New-ItemProperty -Path $k -Name RMInstLoc -PropertyType DWord -Value $Value -Force | Out-Null }
  $v = (Get-ItemProperty -Path $k -ErrorAction SilentlyContinue).RMInstLoc
  "INSTLOC $k RMInstLoc=" + $(if ($null -eq $v) { 'absent' } else { '0x{0:x}' -f $v })
}
