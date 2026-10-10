# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_native_prep.ps1 -User U -Password P -- one-time preparation of the native-NVIDIA baseline box (a throwaway rented
# VM; the password is a throwaway generated per rental and lives only in the box's Winlogon values, as on any autologon
# test machine). Makes an interactive console session appear by itself after every boot (autologon), keeps the screen
# unlocked, stops sleep/screen blank/Windows Update reboots, adds Defender exclusions. Does not reboot.
param([Parameter(Mandatory = $true)][string]$User, [Parameter(Mandatory = $true)][string]$Password)
$ErrorActionPreference = 'Continue'
$notes = @()
try { Set-LocalUser -Name $User -Password (ConvertTo-SecureString $Password -AsPlainText -Force) -PasswordNeverExpires $true -ErrorAction Stop; $notes += 'password set' } catch { $notes += "Set-LocalUser failed: $_" }
$wl = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
Set-ItemProperty $wl AutoAdminLogon '1' -Type String
Set-ItemProperty $wl DefaultUserName $User -Type String
Set-ItemProperty $wl DefaultPassword $Password -Type String
Set-ItemProperty $wl DefaultDomainName $env:COMPUTERNAME -Type String
Set-ItemProperty $wl ForceAutoLogon '1' -Type String
New-Item -Force -Path 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Personalization' | Out-Null
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Personalization' NoLockScreen 1 -Type DWord
New-Item -Force -Path 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\System' | Out-Null
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\System' DisableLockWorkstation 1 -Type DWord
Set-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' InactivityTimeoutSecs 0 -Type DWord
New-Item -Force -Path 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' | Out-Null
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' NoAutoRebootWithLoggedOnUsers 1 -Type DWord
Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' NoAutoUpdate 1 -Type DWord
foreach ($s in 'wuauserv', 'UsoSvc') { try { Stop-Service $s -Force -ErrorAction SilentlyContinue; Set-Service $s -StartupType Disabled -ErrorAction SilentlyContinue } catch { } }
powercfg /change standby-timeout-ac 0 | Out-Null; powercfg /change monitor-timeout-ac 0 | Out-Null; powercfg /change hibernate-timeout-ac 0 | Out-Null; powercfg /hibernate off | Out-Null
try { Add-MpPreference -ExclusionPath 'C:\kfapps', 'C:\kf', 'C:\kfmedia' -ErrorAction Stop; $notes += 'defender exclusions' } catch { $notes += "Defender exclusion failed: $_" }
try { Set-MpPreference -DisableRealtimeMonitoring $true -ErrorAction Stop; $notes += 'defender rt off' } catch { $notes += "Defender rt off failed: $_" }
New-Item -ItemType Directory -Force -Path C:\kf, C:\kfmedia, C:\kfapps | Out-Null
$os = Get-CimInstance Win32_OperatingSystem
$smi = "$env:windir\System32\nvidia-smi.exe"; if (-not (Test-Path $smi)) { $smi = "$env:ProgramFiles\NVIDIA Corporation\NVSMI\nvidia-smi.exe" }
$info = @{
    os = ('{0} build {1}' -f $os.Caption, $os.BuildNumber); notes = $notes
    video = @(Get-CimInstance Win32_VideoController | ForEach-Object { '{0} | {1} | {2} | code {3}' -f $_.Name, $_.DriverVersion, $_.Status, $_.ConfigManagerErrorCode })
    cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name; cores = (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
    ram_gb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
    disk_free_gb = [math]::Round((Get-PSDrive C).Free / 1GB, 1)
    smi = $(if (Test-Path $smi) { (& $smi --query-gpu=name,driver_version,memory.total,pcie.link.gen.current,power.limit --format=csv,noheader 2>&1 | Select-Object -First 1) -as [string] } else { 'no nvidia-smi' })
}
Write-Output ('KFPREP ' + (ConvertTo-Json -InputObject $info -Depth 4 -Compress))
exit 0
