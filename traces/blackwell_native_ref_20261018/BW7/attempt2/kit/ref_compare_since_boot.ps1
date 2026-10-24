# BW7 post-hold read-only query: whole boot includes Code0 and the complete hold.
$ErrorActionPreference='Stop'
$os=Get-CimInstance Win32_OperatingSystem
'REF_OS '+([ordered]@{caption=$os.Caption;version=$os.Version;build=$os.BuildNumber;
 boot=$os.LastBootUpTime.ToUniversalTime().ToString('o');utc=[DateTime]::UtcNow.ToString('o')}|
 ConvertTo-Json -Compress)
$package='C:\ProgramData\VastWindows\nvidia.exe'
if(Test-Path $package){
 'REF_DRIVER_PACKAGE_SHA256 '+(Get-FileHash $package -Algorithm SHA256).Hash
}
$begin=$os.LastBootUpTime
$ids=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=1001,6008,4101;StartTime=$begin} `
 -ErrorAction SilentlyContinue)
$display=@(Get-WinEvent -FilterHashtable @{LogName='System';
 ProviderName='nvlddmkm','Display';Level=1,2;StartTime=$begin} -ErrorAction SilentlyContinue)
'REF_EVENT_COUNTS '+([ordered]@{begin=$begin.ToUniversalTime().ToString('o');
 event1001=@($ids|Where-Object Id -eq 1001).Count;
 event6008=@($ids|Where-Object Id -eq 6008).Count;
 event4101=@($ids|Where-Object Id -eq 4101).Count;displayErrors=$display.Count}|
 ConvertTo-Json -Compress)
@($ids)+@($display)|Sort-Object TimeCreated|Select-Object -First 80|ForEach-Object {
 'REF_EVENT '+([ordered]@{utc=$_.TimeCreated.ToUniversalTime().ToString('o');id=$_.Id;
 provider=$_.ProviderName;level=$_.Level;message=([string]$_.Message).Substring(
 0,[Math]::Min(800,([string]$_.Message).Length))}|ConvertTo-Json -Compress)
}
$files=@()
foreach($dir in 'C:\Windows\Minidump','C:\Windows\LiveKernelReports',
 'C:\ProgramData\Microsoft\Windows\WER\ReportQueue',
 'C:\ProgramData\Microsoft\Windows\WER\ReportArchive'){
 if(Test-Path $dir){$files+=@(Get-ChildItem $dir -Recurse -File -Force -Filter *.dmp)}
}
if(Test-Path C:\Windows\MEMORY.DMP){$files+=Get-Item C:\Windows\MEMORY.DMP}
'REF_DUMP_COUNT '+$files.Count
$files|Select-Object -First 40|ForEach-Object {
 $stream=[IO.File]::Open($_.FullName,'Open','Read','ReadWrite')
 try{$buf=New-Object byte[] 64;$n=$stream.Read($buf,0,64)}finally{$stream.Dispose()}
 $code=$null
 if($n -ge 60 -and [Text.Encoding]::ASCII.GetString($buf,0,8) -eq 'PAGEDU64'){
  $code='0x'+[BitConverter]::ToUInt32($buf,56).ToString('x')
 }
 'REF_DUMP '+([ordered]@{path=$_.FullName;bytes=$_.Length;
 utc=$_.LastWriteTimeUtc.ToString('o');bugcheck=$code}|ConvertTo-Json -Compress)
}
'REF_COMPARE_DONE'
