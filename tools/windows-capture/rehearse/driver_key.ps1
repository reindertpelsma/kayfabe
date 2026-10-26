# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# DEVPKEY_Device_Driver identifies the active device's class-instance registry key.
function Get-KfwcNvidiaDriverKey([string]$InstanceId) {
    $key = (Get-PnpDeviceProperty -InstanceId $InstanceId `
        -KeyName 'DEVPKEY_Device_Driver' -ErrorAction Stop).Data
    $shape = '^\{4d36e968-e325-11ce-bfc1-08002be10318\}\\[0-9]{4}$'
    if ($key -notmatch $shape) { throw 'Unexpected active display driver key shape.' }
    $path = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\' + $key
    $properties = Get-ItemProperty -LiteralPath $path -ErrorAction Stop
    if ($properties.ProviderName -ne 'NVIDIA') { throw 'Active driver provider is not NVIDIA.' }
    return [pscustomobject]@{Path=$path; EnableGpuFirmware=$properties.EnableGpuFirmware}
}
