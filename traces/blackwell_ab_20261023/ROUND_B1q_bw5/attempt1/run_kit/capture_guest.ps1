# B1n private capture (owner 2026-10-24 decision 2), standard-VGA collection boot only:
# stage guest dumps (Minidump, LiveKernelReports incl. WATCHDOG/WATCHDOG4400, WER report
# folders, MEMORY.DMP) and the full event log (every log with records, as .evtx) into
# C:\kf\cap, with a sha256 manifest. Reads and local copies only; nothing leaves the guest
# except through the harness's own scp-from afterwards.
$ErrorActionPreference='Continue'
$ProgressPreference='SilentlyContinue'
$cap='C:\kf\cap'
if(Test-Path $cap){
 Rename-Item -LiteralPath $cap -NewName ('cap_prev_' + [DateTime]::UtcNow.Ticks)
}
New-Item -ItemType Directory -Path $cap | Out-Null
Write-Output "CAPTURE_GUEST_START UTC=$([DateTime]::UtcNow.ToString('o'))"
$roots=[ordered]@{
 'Minidump'='C:\Windows\Minidump';
 'LiveKernelReports'='C:\Windows\LiveKernelReports';
 'WER_ReportQueue'='C:\ProgramData\Microsoft\Windows\WER\ReportQueue';
 'WER_ReportArchive'='C:\ProgramData\Microsoft\Windows\WER\ReportArchive'
}
foreach($name in $roots.Keys){
 $src=$roots[$name]
 if(!(Test-Path -LiteralPath $src)){ Write-Output "NODIR $src"; continue }
 $files=@(Get-ChildItem -LiteralPath $src -Recurse -File -Force -ErrorAction Continue)
 $files | ForEach-Object {
  $rel=$_.FullName.Substring($src.Length).TrimStart('\')
  $dst=Join-Path (Join-Path $cap $name) $rel
  New-Item -ItemType Directory -Force -Path (Split-Path $dst) | Out-Null
  Copy-Item -LiteralPath $_.FullName -Destination $dst -Force -ErrorAction Continue
  if(!(Test-Path -LiteralPath $dst)){ Write-Output "COPY_FAILED $($_.FullName)" }
 }
}
if(Test-Path C:\Windows\MEMORY.DMP){
 New-Item -ItemType Directory -Force -Path "$cap\memory" | Out-Null
 Copy-Item -LiteralPath C:\Windows\MEMORY.DMP -Destination "$cap\memory\MEMORY.DMP" -Force
}
$ev="$cap\evtx"
New-Item -ItemType Directory -Path $ev | Out-Null
$logs=@(Get-WinEvent -ListLog * -ErrorAction SilentlyContinue |
 Where-Object { $_.RecordCount -gt 0 })
$n=0
foreach($l in $logs){
 $n++
 $safe=($l.LogName -replace '[^A-Za-z0-9._-]','_')
 wevtutil epl "$($l.LogName)" "$ev\$('{0:D4}' -f $n)_$safe.evtx" 2>$null
 if($LASTEXITCODE -ne 0){ Write-Output "EVTX_FAILED $($l.LogName) rc=$LASTEXITCODE" }
}
Write-Output "EVTX_LOGS=$($logs.Count)"
foreach($t in 'System','Application'){
 wevtutil qe $t /f:text /rd:false > "$ev\$t.txt" 2>$null
}
$m="$cap\GUEST_MANIFEST.txt"
$all=@(Get-ChildItem -LiteralPath $cap -Recurse -File | Where-Object { $_.FullName -ne $m })
$all | ForEach-Object {
 $h=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
 "GUESTFILE {0} {1} {2}" -f $h,$_.Length,$_.FullName.Substring($cap.Length+1)
} | Set-Content -LiteralPath $m -Encoding ascii
Add-Content -LiteralPath $m -Value 'CAPTURE_GUEST_DONE' -Encoding ascii
$sum=(Get-ChildItem -LiteralPath $cap -Recurse -File | Measure-Object Length -Sum).Sum
Write-Output "CAPTURE_GUEST_DONE bytes=$sum UTC=$([DateTime]::UtcNow.ToString('o'))"
