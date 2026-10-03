# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# gsp_on.ps1 — install the NVIDIA display driver on the kf3 device with GSP FORCED ON, before
# nvlddmkm's first start (docs/design/V3_WINDOWS_DISCOVERY.md; THE_WINDOWS_AXIS.md §1).
#
# Why: kayfabe serves a GSP client only, and a stock GeForce driver on Windows runs monolithic RM
# (owner-measured on Turing and Ada). `EnableGpuFirmware = 1` is MODE_ENABLED without
# ALLOW_FALLBACK (nv-firmware-registry.h), so a GSP init failure is a Code 43, never a silent
# fallback to monolithic RM poking kf3's registers. The value must be in place before nvlddmkm
# starts for the first time on this device.
#
# Order: disable the device → add + install the INF → write EnableGpuFirmware=1 into the device's
# driver (class) key and, belt and braces, the nvlddmkm service key → read both back → enable.
# Everything is logged to C:\kf\gsp_on.log and stdout as `GSP_ON <step> ...` lines.
param(
    [Parameter(Mandatory = $true)][string]$Inf,      # e.g. C:\kf\nv\Display.Driver\nv_dispi.inf
    [string]$DeviceMatch = 'PCI\VEN_10DE&DEV_*',
    [switch]$NoEnable                                 # leave the device disabled (the harness reboots instead)
)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
Start-Transcript -Path C:\kf\gsp_on.log -Append | Out-Null
function Say([string]$s) { Write-Output "GSP_ON $s" }

$dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like $DeviceMatch -and $_.Class -eq 'Display' } | Select-Object -First 1
if (-not $dev) {
    $dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like $DeviceMatch } | Select-Object -First 1
}
if (-not $dev) { Say 'REFUSED no device matches'; Stop-Transcript | Out-Null; exit 2 }
$id = $dev.InstanceId
function Prop([string]$k) { (Get-PnpDeviceProperty -InstanceId $id -KeyName $k -ErrorAction SilentlyContinue).Data }
Say "device $id name='$($dev.FriendlyName)' status=$($dev.Status) problem=$($dev.Problem) inf=$(Prop 'DEVPKEY_Device_DriverInfPath') driverkey=$(Prop 'DEVPKEY_Device_Driver')"

$sig = Get-AuthenticodeSignature (Join-Path (Split-Path $Inf) 'nvlddmkm.sys') -ErrorAction SilentlyContinue
Say "nvlddmkm.sys signature status=$($sig.Status) signer='$($sig.SignerCertificate.Subject)'"

pnputil /disable-device "$id" | ForEach-Object { Say "disable: $_" }
pnputil /add-driver "$Inf" /install | ForEach-Object { Say "add-driver: $_" }
Say "add-driver rc=$LASTEXITCODE"
$infPath = Prop 'DEVPKEY_Device_DriverInfPath'
$key = Prop 'DEVPKEY_Device_Driver'
Say "after install: inf=$infPath driverkey=$key version=$(Prop 'DEVPKEY_Device_DriverVersion')"
if ($infPath -match '^(display|basicdisplay)\.inf$' -or -not $infPath) {
    Say "WARN the device is still on '$infPath': pnputil skipped the disabled device (runbook fallback needed)"
}

$written = @()
if ($key) {
    $classKey = "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$key"
    New-ItemProperty -Path $classKey -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
    $written += $classKey
}
$svcKey = 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
if (-not (Test-Path $svcKey)) { New-Item -Path $svcKey -Force | Out-Null }
New-ItemProperty -Path $svcKey -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
$written += $svcKey
foreach ($k in $written) {
    Say "readback $k EnableGpuFirmware=$((Get-ItemProperty -Path $k -Name EnableGpuFirmware -ErrorAction SilentlyContinue).EnableGpuFirmware)"
}

if (-not $NoEnable) {
    pnputil /enable-device "$id" | ForEach-Object { Say "enable: $_" }
    Start-Sleep -Seconds 20
    $dev = Get-PnpDevice -InstanceId $id
    Say "after enable: name='$($dev.FriendlyName)' status=$($dev.Status) problem=$($dev.Problem) problemstatus=$(Prop 'DEVPKEY_Device_ProblemStatus')"
}
Stop-Transcript | Out-Null
exit 0
