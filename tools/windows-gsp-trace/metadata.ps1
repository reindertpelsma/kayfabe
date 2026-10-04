# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
$os = Get-CimInstance Win32_OperatingSystem
$gpus = @(Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DeviceID -like 'PCI\VEN_10DE*' -and $_.DeviceClass -eq 'DISPLAY' } | Select-Object DeviceID, DeviceName, DriverVersion, DriverDate, InfName, IsSigned)
$driver = Get-CimInstance Win32_SystemDriver -Filter "Name='nvlddmkm'" -ErrorAction SilentlyContinue
$binary = $null
if ($driver) {
    $path = $driver.PathName -replace '^\\SystemRoot', $env:SystemRoot
    $path = $path -replace '^\\\?\?\\',''
    if (Test-Path -LiteralPath $path) { $binary = Get-FileHash -LiteralPath $path -Algorithm SHA256 }
}
$registry = @()
$keys = @('HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm', 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}')
foreach ($key in $keys) {
    if (Test-Path $key) {
        foreach ($item in @(Get-Item $key) + @(Get-ChildItem $key -Recurse -ErrorAction SilentlyContinue)) {
            $value = Get-ItemProperty -LiteralPath $item.PSPath -Name EnableGpuFirmware -ErrorAction SilentlyContinue
            if ($null -ne $value) { $registry += [ordered]@{path=$item.Name;EnableGpuFirmware=$value.EnableGpuFirmware} }
        }
    }
}
[ordered]@{schema='kayfabe-gsp-host/1';captured_utc=(Get-Date).ToUniversalTime().ToString('o');windows=$os.Caption;version=$os.Version;build=$os.BuildNumber;gpus=$gpus;nvlddmkm_sha256=$binary;gsp_registry=$registry;gsp_enabled_proven=$false} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
$smi = Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue
if ($smi) { & $smi.Source -q | Out-File -LiteralPath "$OutputPath.nvidia-smi.txt" -Encoding UTF8 }
