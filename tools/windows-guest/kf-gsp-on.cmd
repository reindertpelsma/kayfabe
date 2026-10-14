@echo off
rem kayfabe: tell the stock NVIDIA Windows driver to use the GSP firmware path (EnableGpuFirmware=1), which is the only path kayfabe serves.
rem Run ONCE as Administrator in the guest, after the NVIDIA driver is installed (or before its first start), then reboot.
rem Writes the value into the display adapter's class key and the nvlddmkm service key (same as scripts/bench/windows/gsp_on.ps1).
powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&DEV_*' -and $_.Class -eq 'Display' } | Select-Object -First 1; if(-not $dev){ Write-Host 'no NVIDIA display device found'; exit 2 }; $k = (Get-PnpDeviceProperty -InstanceId $dev.InstanceId -KeyName DEVPKEY_Device_Driver).Data; if($k){ New-ItemProperty -Path ('HKLM:\SYSTEM\CurrentControlSet\Control\Class\' + $k) -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null; Write-Host ('class key ' + $k + ' set') }; $s='HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'; if(-not (Test-Path $s)){ New-Item -Path $s -Force | Out-Null }; New-ItemProperty -Path $s -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null; Write-Host 'service key set'; Write-Host ('device status: ' + $dev.Status + ' problem: ' + $dev.Problem)"
echo.
echo Done. Reboot Windows now (Start, Power, Restart). The NVIDIA adapter should then show no warning (Code 0).
pause
