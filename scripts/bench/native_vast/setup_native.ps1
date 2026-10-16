# setup_native.ps1 -Pw PASSWORD : throwaway-box setup, run once over ssh as `vast` (admin). Sets the lab account's password and autologon
# (recipe: scripts/bench/diesweep/phase1.ps1.in on origin/codex/sweep-turing-hold-20261016), never sleep, installs Heaven 4.0 Basic
# (installer from the project's usual URL, sha256 computed on the box and printed), makes C:\bench. The caller reboots afterwards.
param([Parameter(Mandatory)][string]$Pw, [string]$User = 'vast')
$ErrorActionPreference = 'Continue'; $ProgressPreference = 'SilentlyContinue'
function Say($s) { Write-Output "SETUP $s" }
Set-LocalUser -Name $User -Password (ConvertTo-SecureString $Pw -AsPlainText -Force); Say "password set for $User"
$wl = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
Set-ItemProperty $wl AutoAdminLogon '1'; Set-ItemProperty $wl DefaultUserName $User; Set-ItemProperty $wl DefaultPassword $Pw; Set-ItemProperty $wl DefaultDomainName '.'
Remove-ItemProperty $wl AutoLogonCount -ErrorAction SilentlyContinue
$pl = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\PasswordLess\Device'
if (-not (Test-Path $pl)) { New-Item -Path $pl -Force | Out-Null }; Set-ItemProperty $pl DevicePasswordLessBuildVersion 0 -Type DWord
powercfg /change standby-timeout-ac 0; powercfg /change monitor-timeout-ac 0; powercfg /change hibernate-timeout-ac 0
Say ("powercfg " + ((powercfg /getactivescheme) -join ' '))
New-Item -ItemType Directory -Force C:\bench, C:\bench\runs, C:\bench\dl | Out-Null
$url = 'https://assets.unigine.com/d/Unigine_Heaven-4.0.exe'; $f = 'C:\bench\dl\Unigine_Heaven-4.0.exe'
if (-not (Test-Path $f)) { Invoke-WebRequest -UseBasicParsing $url -OutFile $f -TimeoutSec 900 }
$h = (Get-FileHash $f -Algorithm SHA256).Hash.ToLower(); Say "HEAVEN_SHA256 $h url=$url size=$((Get-Item $f).Length)"
if (-not (Test-Path C:\kfapps\heaven\bin\Heaven.exe)) {
  $p = Start-Process $f -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/DIR=C:\kfapps\heaven') -Wait -PassThru; Say "heaven installer rc=$($p.ExitCode)" }
Say "heaven exe=$(Test-Path C:\kfapps\heaven\bin\Heaven.exe)"
Say "vcruntime140_1=$(Test-Path C:\Windows\System32\vcruntime140_1.dll)"
