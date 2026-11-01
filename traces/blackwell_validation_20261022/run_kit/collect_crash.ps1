$ErrorActionPreference='Continue'
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
Write-Output "COLLECT now=$([DateTime]::UtcNow.ToString('o')) boot=$($boot.ToUniversalTime().ToString('o'))"
foreach($d in 'C:\Windows\Minidump','C:\Windows\LiveKernelReports','C:\Windows\LiveKernelReports\WATCHDOG'){ if(Test-Path $d){ Get-ChildItem $d -Recurse -File | ForEach-Object { "FILE {0} {1} {2}" -f $_.FullName,$_.Length,$_.LastWriteTimeUtc.ToString('o') } } else { "NODIR $d" } }
if(Test-Path C:\Windows\MEMORY.DMP){ $m=Get-Item C:\Windows\MEMORY.DMP; "FILE {0} {1} {2}" -f $m.FullName,$m.Length,$m.LastWriteTimeUtc.ToString('o') }
$since=$boot.AddHours(-2)
$q=@(
 @{LogName='System';StartTime=$since;Id=1001},@{LogName='System';StartTime=$since;Id=41},@{LogName='System';StartTime=$since;Id=6008},
 @{LogName='System';StartTime=$since;Id=4101},@{LogName='System';StartTime=$since;Id=4117},@{LogName='System';StartTime=$since;Id=4119},
 @{LogName='System';StartTime=$since;ProviderName='nvlddmkm'},@{LogName='System';StartTime=$since;ProviderName='Display'},
 @{LogName='Application';StartTime=$since;Id=1001},@{LogName='Application';StartTime=$since;Id=1000}
)
foreach($h in $q){ $ev=@(); try { $ev=@(Get-WinEvent -FilterHashtable $h -ErrorAction Stop) } catch { "EVTQ_NONE log=$($h.LogName) id=$($h.Id) prov=$($h.ProviderName)" }
 foreach($e in ($ev | Sort-Object TimeCreated)){ $m=($e.Message -replace '\s+',' '); if($m.Length -gt 400){$m=$m.Substring(0,400)}; "EVT log={0} t={1} prov={2} id={3} msg={4}" -f $e.LogName,$e.TimeCreated.ToUniversalTime().ToString('o'),$e.ProviderName,$e.Id,$m } }
Get-ChildItem 'C:\ProgramData\Microsoft\Windows\WER\ReportArchive','C:\ProgramData\Microsoft\Windows\WER\ReportQueue' -Recurse -Filter Report.wer -ErrorAction SilentlyContinue | ForEach-Object { "WER $($_.FullName)"; Get-Content $_.FullName -ErrorAction SilentlyContinue | Select-String -Pattern '^(EventType|Sig\[\d+\]\.(Name|Value)|AppName|FriendlyEventName)=' | ForEach-Object { "  " + $_.Line } }
'COLLECT_DONE'
