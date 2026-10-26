# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# gsp_fw_list.ps1 — which GSP firmware images does the image's NVIDIA package carry (per family), and what are the
# registry values on the present NVIDIA device's class key and the service key? (names + sizes only)
$ErrorActionPreference = 'Continue'
foreach ($d in @(Get-ChildItem C:\Windows\System32\DriverStore\FileRepository -Directory -Filter 'nv_dispi.inf_amd64_*')) {
    Get-ChildItem $d.FullName -Recurse -Include 'gsp*.bin', '*.bin' -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'gsp*' } | ForEach-Object { "GSPFW $($_.Name) bytes=$($_.Length)" }
}
$dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.Class -eq 'Display' } | Select-Object -First 1
if ($dev) {
    $k = (Get-PnpDeviceProperty -InstanceId $dev.InstanceId -KeyName DEVPKEY_Device_Driver).Data
    $p = Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$k"
    "CLASSKEY $k EnableGpuFirmware=$($p.EnableGpuFirmware) status=$($dev.Status) problem=$($dev.Problem)"
    $p.PSObject.Properties | Where-Object { $_.Name -match 'Gsp|Firmware|RM' } | ForEach-Object { "CLASSVAL $($_.Name)=$($_.Value)" }
}
$s = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' -ErrorAction SilentlyContinue
"SERVICE EnableGpuFirmware=$($s.EnableGpuFirmware)"
