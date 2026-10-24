# BW4 REFERENCE (native Windows on the lease): one read-only state snapshot.
$ErrorActionPreference='Continue'
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$v=@(Get-CimInstance Win32_VideoController | Where-Object {$_.PNPDeviceID -like 'PCI\VEN_10DE*'})
$ev=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$boot} `
 -ErrorAction SilentlyContinue)
$nv=@(Get-WinEvent -FilterHashtable @{LogName='System';ProviderName='nvlddmkm';StartTime=$boot} `
 -ErrorAction SilentlyContinue)
foreach($x in $v){
 $r=[ordered]@{utc=[DateTime]::UtcNow.ToString('o');boot=$boot.ToUniversalTime().ToString('o');
  name=$x.Name;driver=$x.DriverVersion;ConfigManagerErrorCode=[int]$x.ConfigManagerErrorCode;
  status=$x.Status;tdr4101=$ev.Count;nvlddmkm_events=$nv.Count;pnp=$x.PNPDeviceID}
 'REF_STATE '+($r|ConvertTo-Json -Compress)
}
$q=@(nvidia-smi -q 2>&1 | ForEach-Object { "$_" })
$i=[array]::IndexOf(($q | ForEach-Object { $_.Trim() }), 'BAR1 Memory Usage')
if($i -ge 0){ $q[$i..($i+3)] | ForEach-Object { 'REF_BAR1 ' + $_.Trim() } }
else { 'REF_BAR1 absent' }
nvidia-smi -L 2>&1 | ForEach-Object { 'REF_SMI_L ' + ("$_" -replace 'GPU-[0-9a-f-]+','GPU-UUID') }
foreach($d in 'C:\Windows\Minidump','C:\Windows\LiveKernelReports'){
 if(Test-Path $d){ Get-ChildItem $d -Recurse -File -Force | ForEach-Object {
  "FILE {0} {1} {2}" -f $_.FullName,$_.Length,$_.LastWriteTimeUtc.ToString('o') } }
 else { "NODIR $d" } }
'REF_STATE_DONE'
