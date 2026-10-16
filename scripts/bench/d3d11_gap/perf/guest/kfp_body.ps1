# kfp_body.ps1 -Tag T -Job C:\kfperf\jobs\T.ps1 : runs one job script with $OUT = C:\kfperf\out\T, log in log.txt, done.flag at the end.
param([string]$Tag, [string]$Job)
$OUT = "C:\kfperf\out\$Tag"; New-Item -ItemType Directory -Force $OUT | Out-Null
Remove-Item "$OUT\done.flag" -ErrorAction SilentlyContinue
"START $((Get-Date).ToString("HH:mm:ss.fff")) user=$env:USERNAME session=$([System.Diagnostics.Process]::GetCurrentProcess().SessionId)" | Out-File "$OUT\log.txt" -Encoding ascii
try { & $Job *>&1 | Out-String -Stream | Add-Content -Path "$OUT\log.txt" -Encoding ascii } catch { "EXC $_" | Add-Content "$OUT\log.txt" }
"END $((Get-Date).ToString("HH:mm:ss.fff"))" | Add-Content "$OUT\log.txt"
"" | Set-Content "$OUT\done.flag"
