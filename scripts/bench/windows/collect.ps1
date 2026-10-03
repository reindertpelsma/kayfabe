# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# collect.ps1 — the Windows half of a kf3 run's evidence, written as TEXT into
# C:\kf\evidence\<Tag>\ for `win_vm.sh collect <tag>` to pull. Dumps are listed, never copied
# (they stay in the guest and are never committed).
param([Parameter(Mandatory = $true)][string]$Tag)
$ErrorActionPreference = 'Continue'
$out = "C:\kf\evidence\$Tag"
New-Item -ItemType Directory -Force -Path $out | Out-Null
function Save([string]$name, [scriptblock]$body) {
    try { & $body 2>&1 | Out-String -Width 400 | Set-Content -Path (Join-Path $out $name) -Encoding utf8 }
    catch { "error: $_" | Set-Content -Path (Join-Path $out $name) -Encoding utf8 }
}

Save 'os.txt' { Get-CimInstance Win32_OperatingSystem | Format-List Caption, Version, BuildNumber, LastBootUpTime; Get-Date -Format o }
Save 'display_devices.txt' { Get-PnpDevice -Class Display | Format-List * }
Save 'nv_device_properties.txt' {
    Get-PnpDevice | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' } | ForEach-Object {
        "== $($_.InstanceId)"
        Get-PnpDeviceProperty -InstanceId $_.InstanceId | Format-Table KeyName, Data -AutoSize -Wrap
    }
}
Save 'pnputil_display.txt' { pnputil /enum-devices /class Display /drivers }
Save 'pnputil_problem.txt' { pnputil /enum-devices /problem }
Save 'video_controller.txt' { Get-CimInstance Win32_VideoController | Format-List Name, PNPDeviceID, DriverVersion, Status, ConfigManagerErrorCode, CurrentHorizontalResolution, CurrentVerticalResolution, AdapterRAM }
Save 'monitors.txt' { Get-PnpDevice -Class Monitor | Format-List FriendlyName, InstanceId, Status; Get-CimInstance -Namespace root\wmi -ClassName WmiMonitorID -ErrorAction SilentlyContinue | Format-List * }
Save 'graphicsdrivers_reg.txt' { reg query HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers }
Save 'nvlddmkm_reg.txt' {
    reg query HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm
    Get-PnpDevice | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' } | ForEach-Object {
        $k = (Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName DEVPKEY_Device_Driver -ErrorAction SilentlyContinue).Data
        if ($k) { "== class key $k"; reg query "HKLM\SYSTEM\CurrentControlSet\Control\Class\$k" /v EnableGpuFirmware }
    }
}
Save 'events_system_nvlddmkm.txt' { Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'nvlddmkm' } -MaxEvents 200 -ErrorAction SilentlyContinue | Format-List TimeCreated, Id, LevelDisplayName, Message }
Save 'events_system_display.txt' { Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Display' } -MaxEvents 100 -ErrorAction SilentlyContinue | Format-List TimeCreated, Id, Message }
Save 'events_system_errors.txt' { Get-WinEvent -FilterHashtable @{ LogName = 'System'; Level = 1, 2, 3 } -MaxEvents 300 -ErrorAction SilentlyContinue | Format-List TimeCreated, ProviderName, Id, LevelDisplayName, Message }
Save 'events_bugcheck.txt' { Get-WinEvent -FilterHashtable @{ LogName = 'System'; Id = 1001 } -MaxEvents 50 -ErrorAction SilentlyContinue | Format-List TimeCreated, ProviderName, Message }
Save 'events_kernel_pnp.txt' { Get-WinEvent -LogName 'Microsoft-Windows-Kernel-PnP/Configuration' -MaxEvents 300 -ErrorAction SilentlyContinue | Where-Object { $_.Message -match 'VEN_10DE|nv_disp|nvlddmkm' -or $_.Id -in 400, 410, 411 } | Format-List TimeCreated, Id, Message }
Save 'events_dxgkrnl.txt' {
    foreach ($l in 'Microsoft-Windows-DxgKrnl-Admin', 'Microsoft-Windows-DxgKrnl-Operational') {
        "== $l"; Get-WinEvent -LogName $l -MaxEvents 200 -ErrorAction SilentlyContinue | Format-List TimeCreated, Id, LevelDisplayName, Message
    }
}
Save 'setupapi_dev_tail.txt' { Get-Content C:\Windows\INF\setupapi.dev.log -Tail 1500 -ErrorAction SilentlyContinue }
Save 'dumps.txt' { Get-ChildItem C:\Windows\LiveKernelReports, C:\Windows\Minidump, C:\Windows\MEMORY.DMP -Recurse -ErrorAction SilentlyContinue | Format-Table FullName, Length, LastWriteTime -AutoSize }
$smi = 'C:\Windows\System32\nvidia-smi.exe'
if (Test-Path $smi) {
    Save 'nvidia_smi_L.txt' { & $smi -L }
    Save 'nvidia_smi_q.txt' { & $smi -q }
} else { 'nvidia-smi.exe not present' | Set-Content (Join-Path $out 'nvidia_smi_q.txt') }
Save 'bitlocker.txt' {
    Get-BitLockerVolume -MountPoint C: | Format-List VolumeStatus, ProtectionStatus, EncryptionPercentage, EncryptionMethod
    manage-bde -status C:
    reg query HKLM\SYSTEM\CurrentControlSet\Control\BitLocker /v PreventDeviceEncryption
}
Save 'secureboot_tpm.txt' {
    "Confirm-SecureBootUEFI=$(Confirm-SecureBootUEFI)"
    Get-Tpm | Format-List
    Get-TpmEndorsementKeyInfo -HashAlgorithm Sha256 | Format-List PublicKeyHash
}
Save 'gsp_on_log.txt' { Get-Content C:\kf\gsp_on.log -ErrorAction SilentlyContinue }
$dx = Join-Path $out 'dxdiag.txt'
Start-Process dxdiag -ArgumentList "/t $dx" -Wait -ErrorAction SilentlyContinue
Get-ChildItem $out | Format-Table Name, Length -AutoSize | Out-String | Write-Output
Write-Output "KF_COLLECT_DONE $out"
