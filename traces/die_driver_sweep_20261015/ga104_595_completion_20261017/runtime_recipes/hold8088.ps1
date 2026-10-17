$oskey=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SOFTWARE\Microsoft\Windows NT\CurrentVersion')
Write-Output ("ACTUAL_OS edition="+$oskey.GetValue('EditionID')+" build="+$oskey.GetValue('CurrentBuildNumber')+" ubr="+$oskey.GetValue('UBR'))
$oskey.Close()
$script:ExpectedVersion='32.0.15.8088'
$script:HoldSeconds=420
$ErrorActionPreference='Stop'

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

$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep'
function AdapterCheck {
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
if($rows[0].driver_version -ne $script:ExpectedVersion -or $rows[0].class_gsp -ne 1) { throw 'Actual driver/GSP binding is not580.88 with GSP1' }
if($rows[0].problem -ne 0) { throw 'NVIDIA adapter must have Code0' }
$dwm=@(Get-Process dwm -ErrorAction SilentlyContinue)
$explorer=@(Get-Process explorer -ErrorAction SilentlyContinue)
Write-Output "DISPLAY_CENSUS_DESKTOP dwm=$($dwm.Count) explorer=$($explorer.Count)"
if(!$dwm.Count -or !$explorer.Count) { throw 'Desktop requires DWM and Explorer' }
Write-Output "DISPLAY_CENSUS_END problem=$($rows[0].problem) $([DateTime]::UtcNow.ToString('o'))"

}
$hashes=@{
'cup2.exe'='eba68ac8f92cf1d65793a6df70d62624d33a8479b3137a45d07d7fd093d79798'
'cup3.exe'='5df837a1914b0fef34e724605b0494c95788a5676f1420863a441a923d0f3f87'
'cup8.exe'='00e69ceb25d4d3af697a79a1bf133fc5f02f5e9125eefde288fa371d856cdef8'
'kf_dxprobe.exe'='2e672bc737a1c6c2332c8b7499cd5459a629d191b27ac4dd9a04a3254c821835'
}

New-Item -ItemType Directory -Force (Join-Path $root 'out') | Out-Null
foreach($name in $hashes.Keys) { if((Get-FileHash (Join-Path $root $name)).Hash.ToLower() -ne $hashes[$name]) { throw "Probe hash mismatch: $name" } }
$script:probeNo=0
function Probe([string]$name,[string[]]$arguments) {
 $script:probeNo++;$out=Join-Path $root "out\probe-$($script:probeNo).log";$err=Join-Path $root "out\probe-$($script:probeNo).err"
 $options=@{FilePath=(Join-Path $root $name);PassThru=$true;RedirectStandardOutput=$out;RedirectStandardError=$err}
 if($arguments.Count) { $options.ArgumentList=$arguments }
 $p=Start-Process @options;$null=$p.Handle
 if(!$p.WaitForExit(180000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue;throw "Probe exceeded180seconds: $name" }
 $p.Refresh()
 if((Get-Item $out).Length -gt 4MB -or (Get-Item $err).Length -gt 4MB) { throw 'Probe log bound exceeded' };$text=Get-Content -Raw $out
 Write-Output $text;Write-Output (Get-Content -Raw $err)
 Write-Output "PROBE_EXIT name=$name args=$arguments rc=$($p.ExitCode)"
 if($p.ExitCode -ne 0) { throw "Probe failed: $name" }
 return
}
function Core {
 AdapterCheck
 foreach($name in 'cup2.exe','cup3.exe','cup8.exe') { Probe $name @() }
 foreach($api in 'd3d11','d3d12') { Probe 'kf_dxprobe.exe' @($api,'--iters','20') }
 AdapterCheck
}
$null=Get-WinEvent -LogName System -MaxEvents 1 -ErrorAction Stop
$since=Get-Date;$clock=[Diagnostics.Stopwatch]::StartNew()
Core
$round=0
while($clock.Elapsed.TotalSeconds -lt $script:HoldSeconds) {
 $round++
 foreach($api in 'd3d11','d3d12') { Probe 'kf_dxprobe.exe' @($api,'--iters','20') }
 AdapterCheck
 $remaining=$script:HoldSeconds-$clock.Elapsed.TotalSeconds
 if($remaining -gt 0) { Start-Sleep -Seconds ([Math]::Min(30,[Math]::Ceiling($remaining))) }
}
Core
if($clock.Elapsed.TotalSeconds -lt $script:HoldSeconds) { throw 'Hold shorter than requested' }
try { $events=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$since} -ErrorAction Stop) }
catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw };$events=@() }
Write-Output "ACCEPTANCE_TDR count=$($events.Count) elapsed_seconds=$([Math]::Round($clock.Elapsed.TotalSeconds,3)) rounds=$round"
if($events.Count) { throw 'TDR during acceptance' }
Write-Output "WINDOWS_SUITE_PASS version=$script:ExpectedVersion hold_seconds=$script:HoldSeconds utc=$([DateTime]::UtcNow.ToString('o'))"
