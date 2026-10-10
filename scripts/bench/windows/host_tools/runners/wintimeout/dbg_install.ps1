$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force C:\kfdbg | Out-Null
$s = @'
$ErrorActionPreference = 'Continue'
"start $(Get-Date -Format o)" | Out-File C:\kfdbg\install.log
try {
  Invoke-WebRequest -UseBasicParsing -Uri 'https://go.microsoft.com/fwlink/?linkid=2286561' -OutFile C:\kfdbg\winsdksetup.exe
  "downloaded $((Get-Item C:\kfdbg\winsdksetup.exe).Length) $(Get-Date -Format o)" | Out-File -Append C:\kfdbg\install.log
  $p = Start-Process C:\kfdbg\winsdksetup.exe -ArgumentList '/features OptionId.WindowsDesktopDebuggers /quiet /norestart /log C:\kfdbg\sdk.log' -Wait -PassThru
  "sdk exit $($p.ExitCode) $(Get-Date -Format o)" | Out-File -Append C:\kfdbg\install.log
} catch { "error $_" | Out-File -Append C:\kfdbg\install.log }
"done $(Get-Date -Format o)" | Out-File -Append C:\kfdbg\install.log
'@
$s | Out-File -Encoding ascii C:\kfdbg\install.ps1
Start-Process powershell.exe -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File C:\kfdbg\install.ps1' -WindowStyle Hidden
"launched"
