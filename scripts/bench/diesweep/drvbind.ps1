# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# drvbind.ps1 — bind the image's own NVIDIA driver package (already in the DriverStore, 580.88) to a NEW die's
# device instance that PnP left without a driver (measured on GB206: CM_PROB_REINSTALL, no driver key, after the
# first boot), then write EnableGpuFirmware=1 into the new class key. SYSTEM via QGA. The driver may start once
# without the flag (monolithic RM, expected Code 43 under kayfabe); the caller then boots a NEW QEMU process.
$ErrorActionPreference = 'Continue'
function Say([string]$s) { Write-Output "BIND $s" }
$dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.InstanceId -notlike '*DEV_22*' } | Select-Object -First 1
if (-not $dev) { Say 'REFUSED no NVIDIA GPU function present'; exit 2 }
$id = $dev.InstanceId; $hw = ($id -split '\\')[1]; $devid = ($hw -split '&')[1]
function Prop([string]$k) { (Get-PnpDeviceProperty -InstanceId $id -KeyName $k -ErrorAction SilentlyContinue).Data }
Say "before hw=$hw status=$($dev.Status) problem=$($dev.Problem) driverkey=$(Prop 'DEVPKEY_Device_Driver') inf=$(Prop 'DEVPKEY_Device_DriverInfPath')"
$infs = @(Get-ChildItem C:\Windows\System32\DriverStore\FileRepository -Recurse -Filter 'nv_disp*.inf' -ErrorAction SilentlyContinue)
Say "driverstore nv_disp INFs: $($infs.Count) $(($infs | ForEach-Object { $_.Directory.Name }) -join ' ')"
$match = $infs | Where-Object { Select-String -Path $_.FullName -Pattern "PCI\\VEN_10DE&$devid" -Quiet } | Select-Object -First 1
if (-not $match) {
    Say "NO_INF_MATCH $devid is in no DriverStore NVIDIA INF (the image's driver does not list this die)"
} else {
    Say "inf $($match.Directory.Name)\$($match.Name) lists $devid"
    pnputil /add-driver "$($match.FullName)" /install | ForEach-Object { Say "pnputil: $_" }
    Say "pnputil rc=$LASTEXITCODE"
}
Start-Sleep -Seconds 30
$key = Prop 'DEVPKEY_Device_Driver'
$d2 = Get-PnpDevice -InstanceId $id
Say "after status=$($d2.Status) problem=$($d2.Problem) class=$($d2.Class) name='$($d2.FriendlyName)' driverkey=$key inf=$(Prop 'DEVPKEY_Device_DriverInfPath') ver=$(Prop 'DEVPKEY_Device_DriverVersion')"
if ($key) {
    $ck = "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$key"
    New-ItemProperty -Path $ck -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
    Say "FLAG $key EnableGpuFirmware=$((Get-ItemProperty -Path $ck -Name EnableGpuFirmware).EnableGpuFirmware)"
} else {
    Say "NO_DRIVER_KEY the device still has no driver"
}
Say "DONE"
