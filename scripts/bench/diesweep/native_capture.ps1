# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Native Windows reference only, on an owned disposable rental. Requires the
# test-signed read-only measurement module and native_oracle.ps1 already staged.
param(
 [ValidatePattern('^[A-Za-z0-9_-]{1,40}$')][string]$CaptureName='capture580',
 [switch]$MailboxOnly,
 [switch]$EnableDisabledDevice
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep'
Set-Location $root
$dir=Join-Path $root $CaptureName
if(Test-Path $dir) { throw 'Capture destination must be new' }
$hashes=@{
 'caps\kfwc_collect.exe'='766dd94a088decfbd76db18b1ad4c18bd819bfa4461797e9dfbfaae755449afd'
 'kf-census.exe'='c2119cc04ee54dde8e056d84fce78178f21634f1477e69f2b4fa8e9acbc46118'
}
foreach($name in $hashes.Keys) {
 if((Get-FileHash -Algorithm SHA256 -LiteralPath $name).Hash.ToLower() -ne $hashes[$name]) { throw "Capture tool hash mismatch: $name" }
 Write-Output "NATIVE_CAPTURE_HASH_OK=$name"
}
$gpu=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($gpu.Count -ne 1) { throw 'Need exactly one NVIDIA display device' }
if($EnableDisabledDevice) {
 $problem=(Get-PnpDeviceProperty -InstanceId $gpu[0].InstanceId -KeyName DEVPKEY_Device_ProblemCode).Data
 if($problem -ne 22) { throw 'Enable replay requires the owned adapter already disabled' }
}
$key=(Get-PnpDeviceProperty -InstanceId $gpu[0].InstanceId -KeyName DEVPKEY_Device_Driver).Data
Set-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$key" -Name EnableGpuFirmware -Type DWord -Value 1
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' -Name EnableGpuFirmware -Type DWord -Value 1
New-Item -Type Directory $dir | Out-Null
$collectorArgs=@($dir,'--seconds','120','--decode-tag','580.65.06')
if($MailboxOnly) { $collectorArgs+='--no-scan' }
$p=Start-Process "$root\caps\kfwc_collect.exe" -ArgumentList $collectorArgs -RedirectStandardOutput "$dir\collector.out" -RedirectStandardError "$dir\collector.err" -PassThru
# Cache the process handle while alive so the exit status remains available.
$null=$p.Handle
Start-Sleep 2
if($p.HasExited) { throw 'Collector exited before the restart' }
Write-Output "NATIVE_CAPTURE_START pid=$($p.Id) $([DateTime]::UtcNow.ToString('o'))"
if($EnableDisabledDevice) {
 & pnputil.exe /enable-device $gpu[0].InstanceId | Out-File "$dir\restart.private.log"
} else {
 & pnputil.exe /restart-device $gpu[0].InstanceId | Out-File "$dir\restart.private.log"
}
if($LASTEXITCODE) { throw "NVIDIA restart/enable failed: $LASTEXITCODE" }
Start-Sleep 15
& nvidia-smi.exe -q | Select-String 'GSP Firmware Version'
& .\native_oracle.ps1 | Tee-Object "$root\native-oracle-gsp580.txt"
if(!$p.WaitForExit(150000)) { throw 'Collector exceeded bounded deadline' }
$p.Refresh()
Write-Output "NATIVE_COLLECTOR_RC=$($p.ExitCode)"
Get-Content "$dir\collector.out"
Get-Content "$dir\collector.err"
if($null -eq $p.ExitCode -or $p.ExitCode -ne 0) { throw 'Collector did not establish exit0' }
& .\kf-census.exe --ogkm-tag 580.65.06 --out "$dir\all" "$dir\capture.kfgspcap"
if($LASTEXITCODE) { throw "Aggregate decode failed: $LASTEXITCODE" }
& .\kf-census.exe scrub --ogkm-tag 580.65.06 --no-default-allow --allow 0x20802801 --allow 0x20802806 --out "$dir\small.scrubbed.kfgspcap" "$dir\capture.kfgspcap"
if($LASTEXITCODE) { throw "Scalar allowlist scrub failed: $LASTEXITCODE" }
& .\kf-census.exe --ogkm-tag 580.65.06 --bodies --out "$dir\small" "$dir\small.scrubbed.kfgspcap"
if($LASTEXITCODE) { throw "Scalar census failed: $LASTEXITCODE" }
Write-Output "NATIVE_CAPTURE_EXIT rc=0 $([DateTime]::UtcNow.ToString('o'))"
