$ErrorActionPreference='Stop'
$key='HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate'
New-Item -Path $key -Force | Out-Null
New-ItemProperty -Path $key -Name ExcludeWUDriversInQualityUpdate -PropertyType DWord -Value 1 -Force | Out-Null
$key='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\DriverSearching'
New-ItemProperty -Path $key -Name SearchOrderConfig -PropertyType DWord -Value 0 -Force | Out-Null
Stop-Service wuauserv -Force -ErrorAction Stop
Set-Service wuauserv -StartupType Disabled
Write-Output 'NATIVE_WINDOWS_DRIVER_UPDATES_DISABLED=1'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\ProgramData\KayfabeSweep\install.ps1 -Action EnableTestSigning
if($LASTEXITCODE) { throw 'Measurement test-signing flag failed' }
Get-Service wuauserv | Select-Object Status,StartType | ConvertTo-Json -Compress
Write-Output "POLICY_EXIT rc=0 $([DateTime]::UtcNow.ToString('o'))"
