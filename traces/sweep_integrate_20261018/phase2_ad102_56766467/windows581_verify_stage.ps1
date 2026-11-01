$ErrorActionPreference='Stop'
$g=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($g.Count -ne 1) { throw 'Require exactly one actual NVIDIA function' }
$id=$g[0].InstanceId
function Prop([string]$k) { (Get-PnpDeviceProperty -InstanceId $id -KeyName $k -ErrorAction Stop).Data }
$v=Prop 'DEVPKEY_Device_DriverVersion'
$inf=Prop 'DEVPKEY_Device_DriverInfPath'
$key=Prop 'DEVPKEY_Device_Driver'
Write-Output "STAGE581_STATE version=$v inf=$inf problem=$($g[0].Problem) driverkey=$key"
if($v -ne '32.0.15.8151' -or $inf -notmatch '^oem[0-9]+\.inf$' -or [int]$g[0].Problem -ne 22 -or !$key) { throw 'Require actual8151 and a disabled device before cold ABI580.94.02; no replay' }
foreach($p in @("HKLM:\SYSTEM\CurrentControlSet\Control\Class\$key",'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm')) {
 $x=(Get-ItemProperty -LiteralPath $p -Name EnableGpuFirmware -ErrorAction Stop).EnableGpuFirmware
 Write-Output "STAGE581_GSP path=$p value=$x"
 if($x -ne 1) { throw 'Both GSP flags must already be1' }
}
Write-Output 'STAGE581_READY=1 actual=32.0.15.8151 device_disabled=1 GSP_flags=1; fresh QEMU580.94.02 then one enable'
