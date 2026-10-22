# diag_events.ps1 — why did the NVIDIA device start fail (Code 43)? (die sweep, 2026-10-14). Text only, no secrets.
$ErrorActionPreference = 'Continue'
function Say([string]$s) { Write-Output "DIAG $s" }
foreach ($d in @(Get-PnpDevice | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.Class -eq 'Display' -and $_.Present })) {
    foreach ($k in 'DEVPKEY_Device_ProblemCode','DEVPKEY_Device_ProblemStatus','DEVPKEY_Device_LastArrivalDate') {
        $v = (Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName $k -ErrorAction SilentlyContinue).Data
        Say "PNP $($d.InstanceId) $k=$v"
    }
}
foreach ($p in 'nvlddmkm', 'Display', 'Microsoft-Windows-Kernel-PnP', 'Microsoft-Windows-DriverFrameworks-UserMode') {
    $ev = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = $p; StartTime = (Get-Date).AddMinutes(-20) } -MaxEvents 12 -ErrorAction SilentlyContinue)
    foreach ($e in $ev) {
        $m = ($e.Message -replace "\s+", ' ')
        if ($m.Length -gt 300) { $m = $m.Substring(0, 300) }
        Say "EVT $p id=$($e.Id) t=$($e.TimeCreated.ToString('HH:mm:ss')) $m"
    }
}
$ev = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; Level = 1,2; StartTime = (Get-Date).AddMinutes(-20) } -MaxEvents 25 -ErrorAction SilentlyContinue)
foreach ($e in $ev) { $m = ($e.Message -replace "\s+", ' '); if ($m.Length -gt 240) { $m = $m.Substring(0, 240) }; Say "ERR $($e.ProviderName) id=$($e.Id) $m" }
# the PnP's own record of the failed start (event 411 carries the NTSTATUS the driver returned)
foreach ($ln in 'Microsoft-Windows-Kernel-PnP/Configuration', 'Microsoft-Windows-Kernel-PnP/Device Management') {
    $ev = @(Get-WinEvent -LogName $ln -MaxEvents 30 -ErrorAction SilentlyContinue | Where-Object { $_.Message -match '10DE|NVIDIA|nvlddmkm' })
    foreach ($e in $ev | Select-Object -First 8) {
        $m = ($e.Message -replace "\s+", ' '); if ($m.Length -gt 400) { $m = $m.Substring(0, 400) }
        Say "PNPLOG $ln id=$($e.Id) $m"
    }
}
foreach ($d in @(Get-PnpDevice | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.Class -eq 'Display' -and $_.Present })) {
    Say ("PNPUTIL " + ((& pnputil.exe /enum-devices /instanceid $d.InstanceId /problem 2>&1) -join ' | '))
}
Say "DONE"
