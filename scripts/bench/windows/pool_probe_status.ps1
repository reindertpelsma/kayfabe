# Read-only guest diagnostics for the Windows pool experiment.
$ErrorActionPreference = 'Continue'
Get-Date -Format o
Get-CimInstance Win32_VideoController |
    Select-Object Name, Status, ConfigManagerErrorCode, PNPDeviceID, DriverVersion |
    ConvertTo-Json -Depth 4
Get-PnpDevice -Class Display |
    Select-Object Status, FriendlyName, InstanceId, Problem |
    ConvertTo-Json -Depth 4
$class = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
Get-ChildItem $class | ForEach-Object {
    Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue |
        Where-Object { $_.ProviderName -match 'NVIDIA' } |
        Select-Object PSChildName, DriverDesc, DriverVersion, EnableGpuFirmware
} | ConvertTo-Json -Depth 4
Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' |
    Select-Object EnableGpuFirmware | ConvertTo-Json
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=(Get-Date).AddMinutes(-30)} -MaxEvents 150 |
    Where-Object { $_.ProviderName -match 'nvlddmkm|Kernel-PnP|Display' } |
    Select-Object TimeCreated, ProviderName, Id, LevelDisplayName, Message |
    ConvertTo-Json -Depth 4
$smi = Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue
if ($smi) { & $smi.Source -L; "NVIDIA_SMI_EXIT=$LASTEXITCODE" }
