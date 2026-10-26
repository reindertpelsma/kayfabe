# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
function Get-KfwcNvidiaDriverKey([string]$InstanceId) {
    $base = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\' +
        '{4d36e968-e325-11ce-bfc1-08002be10318}'
    foreach ($key in Get-ChildItem $base) {
        $v = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue
        if ($v.ProviderName -eq 'NVIDIA') {
            return [pscustomobject]@{Path=$key.PSPath; EnableGpuFirmware=$v.EnableGpuFirmware}
        }
    }
    throw 'No NVIDIA driver key found.'
}
