# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([string]$Root)
$ErrorActionPreference='Stop'
. "$PSScriptRoot\transport.ps1"
$m=Get-Content "$Root\start.json" -Raw | ConvertFrom-Json
$m | Add-Member -NotePropertyName end_qpc -NotePropertyValue ([Diagnostics.Stopwatch]::GetTimestamp())
$files=@{}
function Pack([string]$Path, [long]$Offset=0, [string]$Prefix='') {
    $mem=New-Object IO.MemoryStream
    $gz=New-Object IO.Compression.GZipStream($mem,[IO.Compression.CompressionMode]::Compress,$true)
    if($Prefix) { $b=[Text.Encoding]::UTF8.GetBytes($Prefix+"`n"); $gz.Write($b,0,$b.Length) }
    $f=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    try { $null=$f.Seek($Offset,[IO.SeekOrigin]::Begin); $f.CopyTo($gz) } finally { $f.Dispose(); $gz.Dispose() }
    $s=[Convert]::ToBase64String($mem.ToArray()); $mem.Dispose(); return $s
}
$files['pm_live.csv']=Pack 'C:\kf\hang\pm_live.csv' $m.pm_offset $m.pm_header
$files['trig.log']=Pack 'C:\kf\hang\trig.log' $m.trig_offset
foreach($f in Get-ChildItem "$Root\*" -File | Where-Object {$_.Extension -in @('.tsv','.txt')}) { $files[$f.Name]=Pack $f.FullName }
$procs=@(Get-Process | Where-Object {$_.Name -in @('Heaven','GravityMark','blender','msedge')} | Select-Object Id,Name,Responding)
$etls=@(Get-ChildItem C:\kf\hang\t_*.etl | Select-Object Name,Length,LastWriteTime)
$health=(& "$PSScriptRoot\health.ps1" -Root $Root) | ConvertFrom-Json
ConvertTo-N1Json @{meta=$m; files=$files; processes=$procs; etls=$etls; health=$health}
