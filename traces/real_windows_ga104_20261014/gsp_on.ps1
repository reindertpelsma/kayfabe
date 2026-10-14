# Sets EnableGpuFirmware=1 (DWORD) in the NVIDIA display adapter's class key and in Services\nvlddmkm.
# Registry only; no GPU access. Finds the adapter's class subkey by ProviderName/MatchingDeviceId instead of
# assuming \0000. Prints only the subkey number and the two values (no other registry value is printed).
$base='HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
$sk='HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
$keys = Get-ChildItem $base -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match '^\d{4}$' } | Where-Object {
  $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
  ($p.ProviderName -like '*NVIDIA*') -or ($p.MatchingDeviceId -like 'pci\ven_10de*')
}
if (-not $keys) { throw 'no NVIDIA adapter class key' }
foreach ($k in $keys) {
  New-ItemProperty -Path $k.PSPath -Name EnableGpuFirmware -Value 1 -PropertyType DWord -Force | Out-Null
  "class subkey " + $k.PSChildName + " EnableGpuFirmware=" + (Get-ItemProperty $k.PSPath).EnableGpuFirmware
}
New-ItemProperty -Path $sk -Name EnableGpuFirmware -Value 1 -PropertyType DWord -Force | Out-Null
"service EnableGpuFirmware=" + (Get-ItemProperty $sk).EnableGpuFirmware
