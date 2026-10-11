# vfio_gl_user.ps1 -- run through QGA (SYSTEM) on the VFIO reference guest (2026-10-11): launch kf_glgears.exe in the signed-in
# user's INTERACTIVE session (scheduled task /it as the explorer owner), stdout to C:\kf\gears-out.txt, two whole-screen
# screenshots 3 s apart (C:\kf\gears-a.png / -b.png), result lines to C:\kf\vgl-result.txt. Avoids a variable named $R
# (built-in alias of Invoke-History).
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
$secs = if ($args.Count -gt 0) { [int]$args[0] } else { 40 }
$u = @"
`$ErrorActionPreference = 'Continue'
`$out = 'C:\kf\vgl-result.txt'
function Rw(`$m) { Add-Content `$out ('{0} {1}' -f (Get-Date).ToUniversalTime().ToString('o'), `$m) }
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
function Shot(`$f) { `$b = [System.Windows.Forms.SystemInformation]::VirtualScreen; `$bmp = New-Object System.Drawing.Bitmap `$b.Width, `$b.Height; `$g = [System.Drawing.Graphics]::FromImage(`$bmp); `$g.CopyFromScreen(`$b.Left, `$b.Top, 0, 0, `$bmp.Size); `$bmp.Save(`$f, [System.Drawing.Imaging.ImageFormat]::Png) }
Rw 'USER-TASK start'
Rw ('D: present=' + (Test-Path 'D:\tools\kf_glgears.exe'))
`$p = Start-Process 'D:\tools\kf_glgears.exe' -ArgumentList '--seconds','$secs' -PassThru -RedirectStandardOutput C:\kf\gears-out.txt -RedirectStandardError C:\kf\gears-err.txt
Rw ('GEARS started pid=' + `$p.Id)
Start-Sleep 12
Shot 'C:\kf\gears-a.png'; Rw 'shot a'
Start-Sleep 3
Shot 'C:\kf\gears-b.png'; Rw 'shot b'
`$p.Refresh(); Rw ('GEARS alive=' + (-not `$p.HasExited))
`$p.WaitForExit(120000) | Out-Null; `$p.Refresh()
Rw ('GEARS exit=' + `$(if (`$p.HasExited) { `$p.ExitCode } else { 'still-running' }))
Get-WinEvent -FilterHashtable @{LogName='Application'; Id=1000; StartTime=(Get-Date).AddMinutes(-3)} -ErrorAction SilentlyContinue | ForEach-Object { Rw ('APPCRASH ' + (`$_.Message -replace '[\r\n]+', ' ')) }
Rw 'DONE'
"@
Set-Content -Path C:\kf\vgl_user.ps1 -Value $u -Encoding UTF8
Remove-Item C:\kf\vgl-result.txt, C:\kf\gears-out.txt, C:\kf\gears-err.txt, C:\kf\gears-a.png, C:\kf\gears-b.png -ErrorAction SilentlyContinue
$user = (Get-Process explorer -ErrorAction SilentlyContinue | Select-Object -First 1).UserName
if (-not $user) { $user = "$env:COMPUTERNAME\vast" }
'launch utc=' + (Get-Date).ToUniversalTime().ToString('o') + ' user=' + $user
schtasks /delete /tn kfvgl /f 2>&1 | Out-Null
schtasks /create /tn kfvgl /tr 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\vgl_user.ps1' /sc once /st 00:00 /it /ru $user /rl highest /f 2>&1 | Out-String
schtasks /run /tn kfvgl 2>&1 | Out-String
$end = (Get-Date).AddSeconds($secs + 100)
while ((Get-Date) -lt $end) { if ((Test-Path C:\kf\vgl-result.txt) -and (Select-String -Path C:\kf\vgl-result.txt -Pattern 'DONE' -Quiet)) { break }; Start-Sleep -Milliseconds 500 }
schtasks /delete /tn kfvgl /f 2>&1 | Out-Null
Get-Content C:\kf\vgl-result.txt -ErrorAction SilentlyContinue
'--- gears stdout'; Get-Content C:\kf\gears-out.txt -ErrorAction SilentlyContinue
'--- gears stderr'; Get-Content C:\kf\gears-err.txt -ErrorAction SilentlyContinue
