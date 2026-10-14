# Die identity without board data: PCI device id (vendor+device only, never the subsystem), name, driver
# version, GSP firmware version, architecture. nvidia-smi -q is filtered to three lines on the box; its full
# output (serials, UUID, VBIOS) is never printed or saved.
$smi = 'C:\Windows\System32\nvidia-smi.exe'
& $smi --query-gpu=name,pci.device_id,driver_version --format=csv,noheader
(& $smi -q) | Select-String -Pattern '^\s*(GSP Firmware Version|Product Architecture|Driver Model)\b' | ForEach-Object { $_.Line.Trim() }
Get-CimInstance Win32_VideoController | Where-Object { $_.PNPDeviceID -like 'PCI\VEN_10DE*' } | ForEach-Object {
  $dev = if ($_.PNPDeviceID -match 'VEN_10DE&DEV_([0-9A-F]{4})') { $Matches[1] } else { '?' }
  "pnp DEV_$dev driver $($_.DriverVersion) status $($_.Status)"
}
