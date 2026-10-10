# kfgl.ps1 — runs in the INTERACTIVE session (scheduled task /it). CUDA probe, then the GL gears; result lines to C:\kf\gl-result.txt
$out = 'C:\kf\gl-result.txt'; Remove-Item $out -ErrorAction SilentlyContinue
function R($m) { Add-Content $out ("{0} {1}" -f (Get-Date).ToUniversalTime().ToString('o'), $m) }
for ($i = 0; $i -lt 60 -and -not (Test-Path 'D:\tools\kf_glgears.exe'); $i++) { Start-Sleep 2 }
R ("D: tools present=" + (Test-Path 'D:\tools\kf_glgears.exe'))
foreach ($t in 'cup2.exe') {
  $o = & ("D:\tools\" + $t) 2>&1 | Out-String; R ("$t exit=$LASTEXITCODE out=" + ($o -replace "[\r\n]+", ' | '))
}
$p = Start-Process 'D:\tools\kf_glgears.exe' -PassThru; Start-Sleep 10
$p.Refresh(); R ("kf_glgears alive=" + (-not $p.HasExited) + $(if ($p.HasExited) { " exit=" + $p.ExitCode } else { '' }))
R 'GEARS-UP'
Get-WinEvent -FilterHashtable @{LogName='Application'; Id=1000; StartTime=(Get-Date).AddMinutes(-5)} -ErrorAction SilentlyContinue | ForEach-Object { R ("APPCRASH " + ($_.Message -replace "[\r\n]+", ' ' ).Substring(0, [Math]::Min(300, $_.Message.Length))) }
R 'DONE'
