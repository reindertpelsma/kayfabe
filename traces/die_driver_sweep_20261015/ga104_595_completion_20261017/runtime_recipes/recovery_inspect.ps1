$ErrorActionPreference='Stop'
$r='C:\ProgramData\KayfabeSweep'
$started=Test-Path (Join-Path $r 'driver58088.started')
$exit=Test-Path (Join-Path $r 'driver58088.exit')
$files=@()
foreach($n in 'driver58088.exe','7zr.exe','nvidia58088\setup.exe') {
 $f=Join-Path $r $n
 if(Test-Path $f) { $files+=@{name=$n;bytes=(Get-Item $f).Length} }
}
$rows=@()
$pci=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Enum\PCI')
foreach($device in $pci.GetSubKeyNames()) {
 if($device -notlike 'VEN_10DE&DEV_*'){continue}
 $d=$pci.OpenSubKey($device)
 foreach($instance in $d.GetSubKeyNames()) {
  $i=$d.OpenSubKey($instance)
  if(!(@($i.GetValue('CompatibleIDs')) | Where-Object {$_ -like 'PCI\CC_03*'})){ $i.Close();continue }
  $driver=$i.GetValue('Driver');$version=$null;$before=$null;$after=$null
  if($driver) {
   $k=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SYSTEM\CurrentControlSet\Control\Class\$driver",$true)
   if($k){$version=$k.GetValue('DriverVersion');$before=$k.GetValue('EnableGpuFirmware');$k.SetValue('EnableGpuFirmware',1,[Microsoft.Win32.RegistryValueKind]::DWord);$after=$k.GetValue('EnableGpuFirmware');$k.Close()}
  }
  $rows+=@{pci=($device -split '&SUBSYS_')[0];version=$version;class_gsp_before=$before;class_gsp_after=$after}
  $i.Close()
 }
 $d.Close()
}
$pci.Close()
New-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' EnableGpuFirmware -PropertyType DWord -Value 1 -Force|Out-Null
$result=@{installer_started=$started;installer_exit_marker=$exit;files=$files;nvidia_stored_devices=$rows;service_gsp=1}
Write-Output ('RECOVERY_INSPECT_JSON='+($result|ConvertTo-Json -Depth 5 -Compress))
