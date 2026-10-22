# Export only benchmark text/metrics; keep source file-sharing restrictions intact.
param([string]$Root='C:\Users\Public\codex-VF1')
$ErrorActionPreference='Stop'
$Root=[IO.Path]::GetFullPath($Root).TrimEnd('\')
if (-not $Root.StartsWith('C:\Users\Public\',[StringComparison]::OrdinalIgnoreCase)) {throw 'HARNESS_PRECONDITION: root must be under the public profile'}
$results=Join-Path $Root 'results'
if (-not (Test-Path -LiteralPath $results)) {throw 'HARNESS_PRECONDITION: results directory missing'}
function Read-SharedText([string]$Path) {
 $stream=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
 $reader=New-Object IO.StreamReader($stream)
 try {$reader.ReadToEnd()} finally {$reader.Dispose()}
}
$errors=New-Object 'Collections.Generic.List[string]'
$records=@(Get-ChildItem -LiteralPath $results -File -Recurse | Where-Object {$_.Name -in @('run-log.txt','results.xml','log.txt','stdout.log') -or ($_.Extension -eq '.csv' -and $_.Name -match 'result|score|summary')} | ForEach-Object {
 $file=$_
 if ($file.Length -gt 8388608) {$errors.Add('FILE_TOO_LARGE '+$file.FullName)} else {
  try {@{path=$file.FullName.Substring($results.Length+1);bytes=$file.Length;text=(Read-SharedText $file.FullName)}} catch {$errors.Add('FILE_NOT_READABLE '+$file.FullName+' '+$_.Exception.Message)}
 }
})
$scores=@(Get-ChildItem -LiteralPath $results -Filter result.json -File -Recurse | ForEach-Object {
 $file=$_
 try {
  $meta=(Read-SharedText $file.FullName) | ConvertFrom-Json
  @{run=$file.Directory.Parent.Name;workload=$meta.workloadName;composite=$meta.SPECratio;version=$meta.workloadVersion;source='result.json';renderer=$meta.graphicsRenderer}
 } catch {$errors.Add('SCORE_NOT_READABLE '+$file.FullName+' '+$_.Exception.Message)}
})
$tablePath=Join-Path $Root 'subtest-fps.csv'
$table=if (Test-Path -LiteralPath $tablePath) {Read-SharedText $tablePath} else {''}
@{root=$results;files=$records;scores=$scores;errors=$errors.ToArray();captured=(Get-Date).ToString('o');table=$table} | ConvertTo-Json -Depth 8 -Compress
