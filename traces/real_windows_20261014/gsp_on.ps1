$ck='HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0000'
$sk='HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
New-ItemProperty -Path $ck -Name EnableGpuFirmware -Value 1 -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $sk -Name EnableGpuFirmware -Value 1 -PropertyType DWord -Force | Out-Null
"class=" + (Get-ItemProperty $ck).EnableGpuFirmware + " service=" + (Get-ItemProperty $sk).EnableGpuFirmware
