$ErrorActionPreference='Stop'
Write-Output "DISPLAY_CENSUS_START $([DateTime]::UtcNow.ToString('o'))"
# Read ConfigManager directly: a failed adapter can block Win32_VideoController.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CodexDeviceStatus {
 [DllImport("cfgmgr32.dll", CharSet=CharSet.Unicode, ExactSpelling=true)]
 public static extern uint CM_Locate_DevNodeW(out uint node, string id, uint flags);
 [DllImport("cfgmgr32.dll", ExactSpelling=true)]
 public static extern uint CM_Get_DevNode_Status(out uint status, out uint problem, uint node, uint flags);
}
'@
$pci=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Enum\PCI')
$rows=@()
foreach($device in $pci.GetSubKeyNames()) {
 if($device -notlike 'VEN_10DE&DEV_*') { continue }
 $d=$pci.OpenSubKey($device)
 foreach($instance in $d.GetSubKeyNames()) {
  $i=$d.OpenSubKey($instance)
  $compatible=@($i.GetValue('CompatibleIDs'))
  if(!($compatible | Where-Object { $_ -like 'PCI\CC_03*' })) { $i.Close();continue }
  [uint32]$node=0;[uint32]$status=0;[uint32]$problem=0
  $located=[CodexDeviceStatus]::CM_Locate_DevNodeW([ref]$node,"PCI\$device\$instance",0)
  if($located -ne 0) { $i.Close();continue }
  $result=[CodexDeviceStatus]::CM_Get_DevNode_Status([ref]$status,[ref]$problem,$node,0)
  if($result -ne 0) { throw "ConfigManager status query failed: $result" }
  $driver=$i.GetValue('Driver');$version=$null;$gsp=$null
  if($driver) {
   $key=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SYSTEM\CurrentControlSet\Control\Class\$driver")
   if($key) { $version=$key.GetValue('DriverVersion');$gsp=$key.GetValue('EnableGpuFirmware');$key.Close() }
  }
  $rows+=@{pci=($device -split '&SUBSYS_')[0];driver_version=$version;problem=$problem;status_flags=$status;class_gsp=$gsp}
  $i.Close()
 }
 $d.Close()
}
$pci.Close()
if($rows.Count -ne 1) { throw "Expected one present NVIDIA display function, got $($rows.Count)" }
$rows | ConvertTo-Json -Compress
if($rows[0].driver_version -ne '32.0.15.8088' -or $rows[0].class_gsp -ne 1) { throw 'Actual driver/GSP binding is not580.88 with GSP1' }
$dwm=@(Get-Process dwm -ErrorAction SilentlyContinue)
$explorer=@(Get-Process explorer -ErrorAction SilentlyContinue)
Write-Output "DISPLAY_CENSUS_DESKTOP dwm=$($dwm.Count) explorer=$($explorer.Count)"
Write-Output "DISPLAY_CENSUS_END problem=$($rows[0].problem) $([DateTime]::UtcNow.ToString('o'))"
