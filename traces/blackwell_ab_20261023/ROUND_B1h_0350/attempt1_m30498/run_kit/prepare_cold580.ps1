$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$g=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($g.Count -ne 1) { throw 'Require one actual NVIDIA display function' }
$id=$g[0].InstanceId
function Prop([string]$key) { (Get-PnpDeviceProperty -InstanceId $id -KeyName $key -ErrorAction Stop).Data }
$v=Prop 'DEVPKEY_Device_DriverVersion'
$inf=Prop 'DEVPKEY_Device_DriverInfPath'
$key=Prop 'DEVPKEY_Device_Driver'
Write-Output "COLD580_BIND version=$v inf=$inf driverkey=$key problem=$($g[0].Problem)"
if($v -ne '32.0.15.8088' -or $inf -notmatch '^oem[0-9]+\.inf$' -or !$key) { throw 'NVIDIA580.88 binding is not verified; no installer replay' }
$ck="HKLM:\SYSTEM\CurrentControlSet\Control\Class\$key"
$before=Get-ItemProperty -LiteralPath $ck -Name EnableGpuFirmware -ErrorAction SilentlyContinue
Write-Output "COLD580_GSP_BEFORE class=$($before.EnableGpuFirmware)"
New-ItemProperty -LiteralPath $ck -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
$svc='HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
New-ItemProperty -LiteralPath $svc -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
foreach($p in @($ck,$svc)) {
  $x=(Get-ItemProperty -LiteralPath $p -Name EnableGpuFirmware -ErrorAction Stop).EnableGpuFirmware
  Write-Output "COLD580_GSP_READBACK path=$p value=$x"
  if($x -ne 1) { throw 'GSP readback must be1 before the cold boot' }
}
Write-Output 'COLD580_READY=1; cold QEMU required; this is not a suite PASS'
