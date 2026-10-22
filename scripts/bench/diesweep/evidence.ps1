# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# evidence.ps1 — die-sweep evidence from inside the Windows guest (SYSTEM via QGA). Prints only
# non-identifying fields: no GPU UUID, serial, VBIOS string, MAC or instance-id suffix.
$ErrorActionPreference = 'Continue'
function Say([string]$s) { Write-Output "EVID $s" }
$cs = Get-CimInstance Win32_ComputerSystem
Say "session_user=$($cs.UserName) logonui=$([bool](Get-Process LogonUI -ErrorAction SilentlyContinue)) explorer=$([bool](Get-Process explorer -ErrorAction SilentlyContinue)) dwm=$([bool](Get-Process dwm -ErrorAction SilentlyContinue))"
foreach ($d in @(Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' })) {
    $id = $d.InstanceId
    $k = (Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_Driver -ErrorAction SilentlyContinue).Data
    $ver = (Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_DriverVersion -ErrorAction SilentlyContinue).Data
    $fw = $null; if ($k) { $fw = (Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$k" -Name EnableGpuFirmware -ErrorAction SilentlyContinue).EnableGpuFirmware }
    Say "DEV hw=$(($id -split '\\')[1]) status=$($d.Status) problem=$($d.Problem) class=$($d.Class) name='$($d.FriendlyName)' driverkey=$k ver=$ver EnableGpuFirmware=$fw"
}
foreach ($v in @(Get-CimInstance Win32_VideoController)) {
    Say "VIDEO name='$($v.Name)' status=$($v.Status) mode=$($v.CurrentHorizontalResolution)x$($v.CurrentVerticalResolution) refresh=$($v.CurrentRefreshRate) driver=$($v.DriverVersion)"
}
$smi = (Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue).Source
if (-not $smi) { $smi = 'C:\Windows\System32\nvidia-smi.exe' }
Say "SMI path=$smi"
& $smi 2>&1 | ForEach-Object { "SMI $_" }
& $smi --query-gpu=name,pci.device_id,driver_version,driver_model.current,memory.total,memory.used,pcie.link.gen.current,pcie.link.width.current --format=csv 2>&1 | ForEach-Object { "SMIQ $_" }
& $smi -q -d MEMORY 2>&1 | ForEach-Object { "SMIMEM $_" }
Get-WinEvent -FilterHashtable @{LogName='System'; ProviderName='nvlddmkm'} -MaxEvents 10 -ErrorAction SilentlyContinue |
    ForEach-Object { "NVLOG $($_.TimeCreated.ToString('s')) id=$($_.Id) $(($_.Message -replace '\s+',' ').Substring(0, [Math]::Min(160, ($_.Message -replace '\s+',' ').Length)))" }
Get-WinEvent -FilterHashtable @{LogName='System'; Id=4101} -MaxEvents 5 -ErrorAction SilentlyContinue |
    ForEach-Object { "TDR $($_.TimeCreated.ToString('s')) $($_.ProviderName)" }
Say "DONE"
