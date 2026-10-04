# Read-only failure details beyond the coarse Device Manager code.
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
Get-Date -Format o
Get-PnpDevice -Class Display | ForEach-Object {
    $_ | Select-Object Status, FriendlyName, InstanceId, Problem | ConvertTo-Json
    Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName `
        'DEVPKEY_Device_ProblemCode', 'DEVPKEY_Device_ProblemStatus', `
        'DEVPKEY_Device_DevNodeStatus', 'DEVPKEY_Device_Service' |
        Select-Object KeyName, Type, Data | ConvertTo-Json -Depth 5
}
$since = (Get-Date).AddMinutes(-15)
foreach ($log in @('System', 'Microsoft-Windows-Kernel-PnP/Configuration')) {
    Get-WinEvent -FilterHashtable @{LogName=$log; StartTime=$since} -MaxEvents 250 `
        -ErrorAction SilentlyContinue |
        Where-Object { $_.ProviderName -match 'nvlddmkm|Kernel-PnP|Display|DxgKrnl' } |
        Select-Object TimeCreated, ProviderName, Id, LevelDisplayName, Message |
        ConvertTo-Json -Depth 5
}
