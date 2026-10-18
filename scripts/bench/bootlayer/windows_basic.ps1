# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Read-only oracle INSIDE the disposable Windows guest. Selecting the built-in
# Basic Display driver is an explicit operator step described in README.md.
param([Parameter(Mandatory=$true)][string]$InstanceId, [switch]$ShowClock)
$ErrorActionPreference = 'Stop'
$device = Get-PnpDevice -InstanceId $InstanceId
$driver = Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DeviceID -eq $InstanceId }
if (-not $driver) { throw 'BOOTLAYER_WINDOWS driver metadata missing' }
$service = (Get-PnpDeviceProperty -InstanceId $InstanceId -KeyName 'DEVPKEY_Device_Service').Data
$result = [ordered]@{
    time = (Get-Date).ToUniversalTime().ToString('o')
    instance = $InstanceId
    status = $device.Status
    name = $device.FriendlyName
    provider = $driver.DriverProviderName
    version = $driver.DriverVersion
    service = $service
    basic = ($device.Status -eq 'OK' -and $driver.DriverProviderName -eq 'Microsoft' -and
             $service -eq 'BasicDisplay')
}
$result | ConvertTo-Json
if (-not $result.basic) { throw 'BOOTLAYER_WINDOWS BASIC_DISPLAY_NOT_READY' }
if (-not $ShowClock) { exit 0 }
# A moving clock supplies two distinct visible frames; capture both through QMP
# and the broker. Driver identity alone does not prove a working scanout.
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$form = New-Object System.Windows.Forms.Form
$form.Text = 'BOOTLAYER BASIC DISPLAY RETURN'
$form.Width = 800
$form.Height = 300
$label = New-Object System.Windows.Forms.Label
$label.Dock = 'Fill'
$label.Font = New-Object System.Drawing.Font('Consolas', 28)
$form.Controls.Add($label)
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 200
$timer.Add_Tick({ $label.Text = 'BASIC DISPLAY RETURN ' + (Get-Date).ToString('HH:mm:ss.fff') })
$timer.Start()
try { [void]$form.ShowDialog() } finally { $timer.Stop(); $timer.Dispose(); $form.Dispose() }
