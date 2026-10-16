$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class CodexBinding {
 [DllImport("cfgmgr32.dll",CharSet=CharSet.Unicode,ExactSpelling=true)] public static extern uint CM_Locate_DevNodeW(out uint node,string id,uint flags);
}
'@
function Device {
 $pci=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Enum\PCI');$rows=@()
 foreach($d in $pci.GetSubKeyNames()) {
  if($d -notlike 'VEN_10DE&DEV_*') {continue};$dk=$pci.OpenSubKey($d)
  foreach($i in $dk.GetSubKeyNames()) {
   $ik=$dk.OpenSubKey($i);$compatible=@($ik.GetValue('CompatibleIDs'))
   if(!($compatible | Where-Object {$_ -like 'PCI\CC_03*'})) {$ik.Close();continue}
   [uint32]$node=0;if([CodexBinding]::CM_Locate_DevNodeW([ref]$node,"PCI\$d\$i",0) -ne 0) {$ik.Close();continue}
   $driver=$ik.GetValue('Driver');$version=$null
   if($driver) {$k=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SYSTEM\CurrentControlSet\Control\Class\$driver");if($k){$version=$k.GetValue('DriverVersion');$k.Close()}}
   $rows+=@{device=$d;driver=$driver;version=$version};$ik.Close()
  };$dk.Close()
 };$pci.Close();if($rows.Count -ne 1){throw 'Require one present NVIDIA display device'};return $rows[0]
}
$d=Device;Write-Output "BEFORE_BIND version=$($d.version) driverkey_present=$([bool]$d.driver)"
if($d.version -ne '32.0.15.8088') {
 if($d.version){throw 'Unexpected driver: do not replace blindly'}
 $root='C:\ProgramData\KayfabeSweep';New-Item -ItemType Directory -Force $root|Out-Null;$marker=Join-Path $root 'bind580.started'
 if(Test-Path $marker){throw 'Earlier bind start exists; inspect before replay'}
 $devid=($d.device -split '&')[1]
 $infs=@(Get-ChildItem C:\Windows\System32\DriverStore\FileRepository -Recurse -Filter 'nv_disp*.inf' | Where-Object {Select-String -Path $_.FullName -Pattern "PCI\\VEN_10DE&$devid" -Quiet})
 if(!$infs.Count){throw 'Installed DriverStore has no matching NVIDIA INF'}
 [DateTime]::UtcNow.ToString('o')|Set-Content $marker
 $p=Start-Process pnputil.exe -ArgumentList @('/add-driver',('"'+$infs[0].FullName+'"'),'/install') -PassThru;$null=$p.Handle
 if(!$p.WaitForExit(180000)){throw 'Binding still active; inspect before retry'};$p.Refresh();Write-Output "PNPUTIL_EXIT=$($p.ExitCode)"
 Start-Sleep -Seconds 10;$d=Device
}
if($d.version -ne '32.0.15.8088' -or !$d.driver){throw 'Actual driver binding is not8088'}
$key=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SYSTEM\CurrentControlSet\Control\Class\$($d.driver)",$true)
try{$key.SetValue('EnableGpuFirmware',1,[Microsoft.Win32.RegistryValueKind]::DWord);if($key.GetValue('EnableGpuFirmware') -ne 1){throw 'GSP flag verification failed'}}finally{$key.Close()}
Write-Output 'BIND580_COMPLETE actual8088 classGSP1; cold boot required'
