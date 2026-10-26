# Guest collector: print ONLY the public dump-header bytes (first 4016) of each dump, as hex, plus file size.
$ErrorActionPreference='Continue'
foreach($d in 'C:\Windows\Minidump','C:\Windows\LiveKernelReports'){
  if(Test-Path $d){ Get-ChildItem $d -Recurse -File -Filter *.dmp | ForEach-Object {
    $fs=[IO.File]::Open($_.FullName,'Open','Read','ReadWrite'); try{ $b=New-Object byte[] 4016; $n=$fs.Read($b,0,4016) } finally { $fs.Dispose() }
    if($n -eq 4016){ "HDR {0} {1} {2}" -f ($_.FullName -replace '\\','/'),$_.Length,(($b|ForEach-Object{$_.ToString('x2')}) -join '') }
  } }
}
'HDR_DONE'
