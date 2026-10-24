$ErrorActionPreference='Stop'
$roots=@('C:\ProgramData\Microsoft\Windows\WER\ReportArchive','C:\ProgramData\Microsoft\Windows\WER\ReportQueue')
foreach($root in $roots){
 if(!(Test-Path -LiteralPath $root)){continue}
 foreach($f in @(Get-ChildItem -LiteralPath $root -Recurse -Filter Report.wer -ErrorAction Stop)){
  $map=@{}
  foreach($line in Get-Content -LiteralPath $f.FullName -ErrorAction Stop){
   if($line -match '^(EventType|EventTime|ReportIdentifier|Sig\[\d+\]\.(?:Name|Value))=(.*)$'){$map[$matches[1]]=$matches[2]}
  }
  $utc=$null
  if([string]$map['EventTime'] -match '^[1-9][0-9]{15,19}$'){$utc=[DateTime]::FromFileTimeUtc([int64]$map['EventTime']).ToString('o')}
  [ordered]@{path=$f.FullName;lastWriteUtc=$f.LastWriteTimeUtc.ToString('o');eventUtc=$utc;fields=$map}|ConvertTo-Json -Depth 4 -Compress
 }
}
'WER_TIMES_DONE'
