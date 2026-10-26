# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([Parameter(Mandatory)][string]$Helper)
$ErrorActionPreference = 'Stop'
. $Helper
$script:classId = '{4d36e968-e325-11ce-bfc1-08002be10318}'
$script:provider = 'NVIDIA'
$script:keyData = ''
$script:reads = @()
function Get-ChildItem { throw 'Protected Properties sibling must not be enumerated.' }
function Get-PnpDeviceProperty {
    [CmdletBinding()]param([string]$InstanceId, [string]$KeyName)
    if ($InstanceId -ne 'OWN_GPU' -or $KeyName -ne 'DEVPKEY_Device_Driver') {
        throw 'Wrong device property request.'
    }
    return [pscustomobject]@{Data=$script:keyData}
}
function Get-ItemProperty {
    [CmdletBinding()]param([string]$LiteralPath)
    $script:reads += $LiteralPath
    return [pscustomobject]@{ProviderName=$script:provider; EnableGpuFirmware=0}
}
$failed = 0
foreach ($index in '0000','0012') {
    $script:keyData = "$script:classId\$index"
    $script:reads = @()
    try {
        $r = Get-KfwcNvidiaDriverKey 'OWN_GPU'
        $expected = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\' + $script:keyData
        if ($r.Path -ne $expected -or $script:reads.Count -ne 1 -or
            $script:reads[0] -ne $expected) { throw 'Wrong active driver key.' }
        "ok active_driver_$index"
    } catch {
        "FAIL active_driver_$index"
        $failed++
    }
}
$script:keyData = 'WrongClass\0000'
$script:reads = @()
try {
    $null = Get-KfwcNvidiaDriverKey 'OWN_GPU'
    throw 'unexpected acceptance'
} catch {
    if ($_.Exception.Message -eq 'unexpected acceptance' -or $script:reads.Count) {
        'FAIL rejects_wrong_class_before_read'
        $failed++
    } else { 'ok rejects_wrong_class_before_read' }
}
$script:keyData = "$script:classId\0000"
$script:provider = 'OtherVendor'
try {
    $null = Get-KfwcNvidiaDriverKey 'OWN_GPU'
    throw 'unexpected acceptance'
} catch {
    if ($_.Exception.Message -eq 'unexpected acceptance') {
        'FAIL rejects_wrong_provider'
        $failed++
    } else { 'ok rejects_wrong_provider' }
}
if ($failed) { exit 1 }
