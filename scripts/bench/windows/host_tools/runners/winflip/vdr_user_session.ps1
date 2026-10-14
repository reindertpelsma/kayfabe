# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# vdr_user_session.ps1 -- run through QGA as SYSTEM (2026-10-08, VFIO DVI reference): run work INSIDE the logged-on
# user's interactive desktop (autologon account 'kf'): open a visible command prompt that runs nvidia-smi, run the
# D3D11 clear probe (C:\kf\d3d11_clear_probe.ps1, staged first) and nvidia-smi in that session, then take a
# screenshot of the whole virtual screen to C:\kf\shot-user.png. A scheduled task with an Interactive logon
# is the only way from session 0 into the user's desktop. Prints the outputs. Touches only C:\kf and its task.
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
$u = @'
$ErrorActionPreference = 'Continue'
Start-Process cmd.exe -ArgumentList '/k title kf nvidia-smi & nvidia-smi & echo. & nvidia-smi -L & echo %DATE% %TIME%'
Start-Sleep 6
& 'C:\Windows\System32\nvidia-smi.exe' > C:\kf\smi-user.txt 2>&1
"exit=$LASTEXITCODE utc=$((Get-Date).ToUniversalTime().ToString('o'))" | Add-Content C:\kf\smi-user.txt
powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\d3d11_clear_probe.ps1 > C:\kf\d3d-user.txt 2>&1
Start-Sleep 2
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
$bmp.Save('C:\kf\shot-user.png', [System.Drawing.Imaging.ImageFormat]::Png)
"screens: " + (([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { $_.DeviceName + ' ' + $_.Bounds + ' primary=' + $_.Primary }) -join '; ') | Set-Content C:\kf\screens-user.txt
'@
Set-Content -Path C:\kf\vdr_user.ps1 -Value $u -Encoding UTF8
Remove-Item C:\kf\smi-user.txt, C:\kf\d3d-user.txt, C:\kf\shot-user.png, C:\kf\screens-user.txt -ErrorAction SilentlyContinue
$a = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument '-NoProfile -ExecutionPolicy Bypass -WindowStyle Minimized -File C:\kf\vdr_user.ps1'
$p = New-ScheduledTaskPrincipal -UserId 'kf' -LogonType Interactive -RunLevel Highest
Unregister-ScheduledTask -TaskName kfvdr -Confirm:$false -ErrorAction SilentlyContinue
Register-ScheduledTask -TaskName kfvdr -Action $a -Principal $p | Out-Null
'USER start ' + (Get-Date).ToUniversalTime().ToString('o')
Start-ScheduledTask -TaskName kfvdr
for ($i = 0; $i -lt 120 -and -not (Test-Path C:\kf\screens-user.txt); $i++) { Start-Sleep 1 }
'USER task state ' + (Get-ScheduledTask -TaskName kfvdr).State + ' after ' + $i + ' s; ' + (Get-Date).ToUniversalTime().ToString('o')
'USER quser ' + ((quser 2>&1 | Out-String).Trim() -replace '\s+', ' ')
Get-Content C:\kf\smi-user.txt -ErrorAction SilentlyContinue | ForEach-Object { 'SMI ' + $_ }
Get-Content C:\kf\d3d-user.txt -ErrorAction SilentlyContinue | ForEach-Object { 'D3DU ' + $_ }
Get-Content C:\kf\screens-user.txt -ErrorAction SilentlyContinue | ForEach-Object { 'SCR ' + $_ }
'USER shot ' + (Get-Item C:\kf\shot-user.png -ErrorAction SilentlyContinue).Length
