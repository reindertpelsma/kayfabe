$ErrorActionPreference='Continue'
$since=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
Write-Output "EVENTS_SINCE boot=$($since.ToUniversalTime().ToString('o')) now=$([DateTime]::UtcNow.ToString('o'))"
$q=@()
foreach($p in @('nvlddmkm','Display','Microsoft-Windows-Kernel-PnP','Microsoft-Windows-DriverFrameworks-UserMode','Microsoft-Windows-UserPnp','Microsoft-Windows-Dxgkrnl')){ $q+=@{LogName='System';StartTime=$since;ProviderName=$p} }
$q+=@{LogName='System';StartTime=$since;Id=4101}
foreach($p in @('Application Error','Windows Error Reporting','nvlddmkm')){ $q+=@{LogName='Application';StartTime=$since;ProviderName=$p} }
foreach($h in $q){
 $ev=@()
 try { $ev=@(Get-WinEvent -FilterHashtable $h -ErrorAction Stop) } catch { Write-Output ("EVTQ_NONE log={0} prov={1} id={2} why={3}" -f $h.LogName,$h.ProviderName,$h.Id,$_.FullyQualifiedErrorId.Split(',')[0]) }
 foreach($e in ($ev | Sort-Object TimeCreated)) {
  $m=($e.Message -replace '\s+',' '); if($m.Length -gt 240){$m=$m.Substring(0,240)}
  Write-Output ("EVT log={0} t={1} prov={2} id={3} lvl={4} msg={5}" -f $e.LogName,$e.TimeCreated.ToUniversalTime().ToString('o'),$e.ProviderName,$e.Id,$e.LevelDisplayName,$m)
 }
}
$g=@(Get-PnpDevice -PresentOnly | Where-Object InstanceId -like 'PCI\VEN_10DE*')
foreach($d in $g){ Write-Output ("PNP id={0} class={1} status={2} problem={3}" -f $d.InstanceId,$d.Class,$d.Status,$d.Problem)
 try { $ps=(Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_ProblemStatus -ErrorAction Stop).Data; Write-Output ("PNP_PROBLEM_STATUS {0}" -f $ps) } catch { Write-Output "PNP_PROBLEM_STATUS n/a" } }
Write-Output 'EVENTS_DONE'
