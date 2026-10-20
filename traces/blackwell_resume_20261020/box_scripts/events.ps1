$ErrorActionPreference='Stop'
$since=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
Write-Output "EVENTS_SINCE boot=$($since.ToUniversalTime().ToString('o'))"
$q=@(
 @{LogName='System';StartTime=$since;ProviderName=@('nvlddmkm','Display','Microsoft-Windows-Kernel-PnP','Microsoft-Windows-DriverFrameworks-UserMode','Microsoft-Windows-UserPnp')},
 @{LogName='System';StartTime=$since;Id=4101},
 @{LogName='Application';StartTime=$since;ProviderName=@('Application Error','Windows Error Reporting','NVIDIA OpenGL Driver','nvlddmkm')}
)
foreach($h in $q){
 try { $ev=@(Get-WinEvent -FilterHashtable $h -ErrorAction Stop) } catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*'){throw}; $ev=@() }
 foreach($e in ($ev | Sort-Object TimeCreated)) {
  $m=($e.Message -replace '\s+',' '); if($m.Length -gt 240){$m=$m.Substring(0,240)}
  Write-Output ("EVT log={0} t={1} prov={2} id={3} lvl={4} msg={5}" -f $e.LogName,$e.TimeCreated.ToUniversalTime().ToString('o'),$e.ProviderName,$e.Id,$e.LevelDisplayName,$m)
 }
}
$g=@(Get-PnpDevice -PresentOnly | Where-Object InstanceId -like 'PCI\VEN_10DE*')
foreach($d in $g){ Write-Output ("PNP id={0} class={1} status={2} problem={3}" -f $d.InstanceId,$d.Class,$d.Status,$d.Problem)
 try { $ps=(Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_ProblemStatus -ErrorAction Stop).Data; Write-Output ("PNP_PROBLEM_STATUS 0x{0:x8}" -f [uint32]$ps) } catch { Write-Output "PNP_PROBLEM_STATUS n/a" } }
Write-Output 'EVENTS_DONE'
