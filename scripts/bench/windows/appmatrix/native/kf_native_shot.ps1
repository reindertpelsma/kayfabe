# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_native_shot.ps1 -- native baseline only (no QMP screendump on a rented box): capture the signed-in user's desktop
# to C:\kf\shot_native.png by running a one-shot task with an Interactive principal in that user's session.
$ErrorActionPreference = 'Continue'
$user = (Get-CimInstance Win32_ComputerSystem).UserName
if (-not $user) { Write-Output 'KFSHOT fail no-interactive-user'; exit 3 }
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
$out = 'C:\kf\shot_native.png'
Remove-Item -Force -ErrorAction SilentlyContinue $out
$code = @'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class KfDpi { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }'
[void][KfDpi]::SetProcessDPIAware()
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
$bmp.Save('C:\kf\shot_native.tmp.png', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Move-Item -Force 'C:\kf\shot_native.tmp.png' 'C:\kf\shot_native.png'
'@
Set-Content -Path C:\kf\shot_task.ps1 -Value $code -Encoding ASCII
try { Unregister-ScheduledTask -TaskName kf_native_shot -Confirm:$false -ErrorAction SilentlyContinue } catch { }
$act = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\kf\shot_task.ps1'
$pr = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
Register-ScheduledTask -TaskName kf_native_shot -Action $act -Principal $pr -Force | Out-Null
Start-ScheduledTask -TaskName kf_native_shot
for ($i = 0; $i -lt 40 -and -not (Test-Path $out); $i++) { Start-Sleep -Milliseconds 500 }
Unregister-ScheduledTask -TaskName kf_native_shot -Confirm:$false -ErrorAction SilentlyContinue
if (Test-Path $out) { Write-Output 'KFSHOT ok'; exit 0 } else { Write-Output 'KFSHOT fail no-file'; exit 4 }
