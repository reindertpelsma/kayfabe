#!/bin/bash
# SPDX-License-Identifier: ISC
# License: https://github.com/reindertpelsma/windows-on-vast/blob/main/LICENSE
set -Eeuo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
umask 077
work=/var/lib/vast-reinstall
render_only=false
if [[ ${1:-} == --render-only && $# == 2 ]]; then
  work=$2
  render_only=true
else
  [[ $# == 0 && $EUID == 0 && $(uname -m) == x86_64 ]]
  [[ $(systemd-detect-virt --vm) == kvm ]]
  [[ $(systemd-detect-virt --container || true) == none ]]
  exec 9>/run/vast-reinstall-template.lock
  flock -n 9 || exit 0
  [[ ! -e "$work/started" ]] || exit 0
  systemctl is-active --quiet vast-reinstall.service && exit 0
  printf '\n*** DO NOT USE: this disk is being ERASED for Windows.\n*** Install nothing. If stuck, destroy it and rent again.\n\n' >/etc/motd
fi
mkdir -p "$work/payload"
chmod 700 "$work" "$work/payload"
cat >"$work/worker.sh" <<'WORKER'
#!/bin/bash
set -Eeuo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin DEBIAN_FRONTEND=noninteractive
umask 077
cd -- "$(dirname -- "$0")"
export VAST_REINSTALL_WORK=$PWD
exec 8>worker.lock
flock -n 8 || exit 0
[[ ! -e started ]] || exit 0
trap 'echo "FAILED line $LINENO; see journalctl -u vast-reinstall" >&2' ERR
[[ $EUID == 0 ]]
ready=false
for ((i=0; i<150; i++)); do
  if [[ -s /root/.ssh/authorized_keys ]] && ssh-keygen -lf /root/.ssh/authorized_keys >/dev/null 2>&1 &&
   sshd -t && { systemctl is-active --quiet ssh || systemctl is-active --quiet sshd; }; then
    ready=true
    break
  fi
  sleep 2
done
$ready || { echo 'SSH/keys unavailable.' >&2; exit 1; }
touch started
apt-get -o DPkg::Lock::Timeout=300 update
apt-get -o DPkg::Lock::Timeout=300 install -y --no-install-recommends git ca-certificates curl python3
sshd -T >sshd-effective.txt
python3 - <<'PY'
import json, pathlib, re, subprocess, tempfile
p = pathlib.Path('payload')
keys = []
for line in pathlib.Path('/root/.ssh/authorized_keys').read_text().splitlines():
  line = line.strip()
  if not line or line.startswith('#'): continue
  if not re.fullmatch(r'(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(?:256|384|521)) [A-Za-z0-9+/]+={0,2}(?: .*)?', line):
    raise SystemExit('Unsupported SSH key options/type')
  kind, body, *_ = line.split()
  with tempfile.NamedTemporaryFile(mode='w+') as f:
    f.write(kind + ' ' + body + '\n'); f.flush()
    subprocess.run(['ssh-keygen', '-lf', f.name], check=True, stdout=subprocess.DEVNULL)
  keys.append(kind + ' ' + body)
ports = {int(x.split()[1]) for x in pathlib.Path('sshd-effective.txt').read_text().splitlines() if x.startswith('port ')}
if not keys or len(ports) != 1: raise SystemExit('Need keys and one SSH port')
port = ports.pop()
(p/'inventory.json').write_text(json.dumps({'ssh': {'port': port, 'authorized_keys': keys}}))
(p/'authorized_keys').write_text('\n'.join(keys) + '\n')
PY
port=$(python3 -c 'import json; print(json.load(open("payload/inventory.json"))["ssh"]["port"])')
root_part=$(findmnt -nro SOURCE /)
[[ -b $root_part && $(lsblk -dnro TYPE "$root_part") == part ]]
disk=/dev/$(lsblk -nro PKNAME "$root_part")
[[ -b $disk && $(lsblk -dnro TYPE "$disk") == disk ]]
[[ $(blockdev --getsize64 "$disk") -ge 100000000000 ]]
[[ $(df -B1 --output=avail / | tail -1) -ge 2147483648 ]]
echo "Erasing $disk."
upstream=80c3d5e175f39c2d2bbd267cd583842140154140
git init upstream
git -C upstream fetch --depth=1 https://github.com/bin456789/reinstall.git "$upstream"
git -C upstream checkout --detach FETCH_HEAD
curl --fail --location --proto '=https' --proto-redir '=https' --retry 5 --connect-timeout 30 \
  -o payload/OpenSSH-Win64.msi https://github.com/PowerShell/Win32-OpenSSH/releases/download/10.0.0.0p2-Preview/OpenSSH-Win64-v10.0.0.0.msi
printf '%s  %s\n' ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9 payload/OpenSSH-Win64.msi | sha256sum -c -
ssh-keygen -q -t ed25519 -N '' -f payload/ssh_host_ed25519_key
echo 'Save this WINDOWS SSH HOST KEY before reboot:'
cat payload/ssh_host_ed25519_key.pub
ssh-keygen -lf payload/ssh_host_ed25519_key.pub
python3 patch.py upstream "$PWD"
bash -n reinstall.sh
printf '%s\r\n' '@echo off' 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%ProgramData%\VastWindows\bootstrap.ps1" -Register' 'exit /b %errorlevel%' >payload/vast-bootstrap.cmd
bash reinstall.sh windows --image-name 'Windows 11 Enterprise LTSC 2024 Evaluation' \
  --iso 'https://go.microsoft.com/fwlink/?linkid=2289029&clcid=0x409&culture=en-us&country=us' \
  --username vast --ssh-key "$PWD/payload/authorized_keys" --ssh-port "$port" --target-disk "$disk" </dev/null
touch armed
sync
echo Rebooting.
systemctl reboot
WORKER
cat >"$work/patch.py" <<'PATCHER'
import pathlib, subprocess, sys
source, work = map(pathlib.Path, sys.argv[1:])
revision = '80c3d5e175f39c2d2bbd267cd583842140154140'
def load(name):
    return subprocess.check_output(['git','-C',str(source),'show',revision+':'+name]).decode()
def once(text, old, new):
    if text.count(old) != 1:
        raise SystemExit('Upstream hook changed: ' + repr(old))
    return text.replace(old, new, 1)
front = load('reinstall.sh')
trans = load('trans.sh')
base = 'https://raw.githubusercontent.com/bin456789/reinstall/'
front = once(front, 'confhome='+base+'main', 'confhome='+base+revision)
front = once(front, 'confhome_cn=https://cnb.cool/bin456789/reinstall/-/git/raw/main', 'confhome_cn=')
front = once(front, '''    windows)
        [ -z "$ssh_keys" ] || error_and_exit "not support set ssh key for $distro."
        ;;
''', '')
front = once(front, '        cat <<<"$ssh_keys" >$initrd_dir/configs/ssh_keys', '''        cat <<<"$ssh_keys" >$initrd_dir/configs/ssh_keys
        if [ "$distro" = windows ]; then
            password=$(openssl rand -hex 32)
            save_password $initrd_dir/configs
            unset password
        fi''')
front = once(front, '    curl -Lo $initrd_dir/trans.sh $confhome/trans.sh', '''    cp "$VAST_REINSTALL_WORK/trans.sh" "$initrd_dir/trans.sh"
    mkdir -p "$initrd_dir/configs/vast-windows"
    cp -a "$VAST_REINSTALL_WORK/payload/." "$initrd_dir/configs/vast-windows/"''')
trans = once(trans, '    if $use_gpo; then\n', '''    vast_dir=$(get_path_in_correct_case "$os_dir/ProgramData/VastWindows")
    mkdir -p "$vast_dir"
    cp -r /configs/vast-windows/. "$vast_dir/"
    cp /configs/vast-windows/vast-bootstrap.cmd "$os_dir/vast-bootstrap.cmd"
    bats="$bats vast-bootstrap.cmd"
    if $use_gpo; then
''')
trans = once(trans, '    download $confhome/windows.xml /tmp/autounattend.xml', '''    download $confhome/windows.xml /tmp/autounattend.xml
    sed -i 's/<fDenyTSConnections>false/<fDenyTSConnections>true/' /tmp/autounattend.xml''')
trans = once(trans, '    use_default_rdp_port=$(is_need_change_rdp_port && echo false || echo true)',
             '    use_default_rdp_port=false')
(work/'reinstall.sh').write_text(front)
(work/'trans.sh').write_text(trans)
PATCHER
cat >"$work/payload/bootstrap.ps1" <<'POWERSHELL'
param([switch]$Register)
$ErrorActionPreference = 'Stop'
# Exclude Windows Update driver packages before scheduling the pinned install.
# https://learn.microsoft.com/en-us/windows/deployment/update/waas-configure-wufb
# An earlier update may already be in flight; verify the actual build in each oracle.
$wuPolicy = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate'
New-Item -Path $wuPolicy -Force | Out-Null
New-ItemProperty -Path $wuPolicy -Name ExcludeWUDriversInQualityUpdate -PropertyType DWord -Value 1 -Force | Out-Null
$root = 'C:\ProgramData\VastWindows'
Set-Location $root
function Run($file, [string[]]$arguments, $success = @(0), $seconds = 3600) {
  $p = New-Object Diagnostics.Process
  $p.StartInfo.FileName = $file
  $p.StartInfo.Arguments = ($arguments | ForEach-Object { $_ -replace '^(.*\s.*)$','"$1"' }) -join ' '
  $p.StartInfo.UseShellExecute = $false
  try {
    if (!$p.Start()) { throw "Cannot start $file" }
    if (!$p.WaitForExit($seconds * 1000)) {
      & taskkill.exe /PID $p.Id /T /F | Out-Null
      $null = $p.WaitForExit(30000)
      throw "Timed out: $file"
    }
    $code = $p.ExitCode
    if ($null -eq $code -or $code -notin $success) { throw "$file exited $code" }
    return $code
  } finally { $p.Dispose() }
}
function Save {
  $state | ConvertTo-Json -Depth 6 | Set-Content state.tmp -Encoding UTF8
  Move-Item state.tmp state.json -Force
}
function Reboot {
  if ($state.reboots -ge 2) { throw 'GPU setup exceeded two reboots' }
  $state.reboots++; Save
  $null = Run shutdown.exe @('/r','/t','30','/c','Vast Windows GPU setup')
}
function Download($name) {
  $row = Get-Content packages.txt | Where-Object { $_.StartsWith("$name ") }
  $file, $hash, $url = $row -split ' '
  if (!(Test-Path $file) -or (Get-FileHash $file).Hash -ne $hash) {
    Write-Output "DOWNLOAD $url" | Out-Host
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing $url -OutFile "$file.part" -TimeoutSec 1800
    if ((Get-FileHash "$file.part").Hash -ne $hash) { throw "Bad checksum: $file" }
    Move-Item "$file.part" $file -Force
  }
  return "$root\$file"
}
function NvidiaSignature($file) {
  $s = Get-AuthenticodeSignature $file
  if ($s.Status -ne 'Valid' -or $s.SignerCertificate.Subject -notlike '*NVIDIA*') {
    throw "Invalid NVIDIA signature: $file"
  }
}
if ($Register) {
  $null = Run icacls.exe @($root,'/inheritance:r','/grant:r','*S-1-5-18:(OI)(CI)F','*S-1-5-32-544:(OI)(CI)F')
  $action = New-ScheduledTaskAction -Execute powershell.exe -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File $root\bootstrap.ps1"
  $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 3) -RestartCount 2 -RestartInterval (New-TimeSpan -Minutes 1)
  $principal = New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest
  Register-ScheduledTask VastWindowsSetup -Action $action -Trigger (New-ScheduledTaskTrigger -AtStartup) -Settings $settings -Principal $principal -Force | Out-Null
  Start-ScheduledTask VastWindowsSetup
  exit
}
Start-Transcript "$root\setup.log" -Append | Out-Null
$ProgressPreference = 'SilentlyContinue'
$state = [pscustomobject]@{runs=0; failures=0; reboots=0; ssh=$false; driver=$false; cuda=$false; complete=$false; error=$null}
if (Test-Path state.json) { $state = Get-Content -Raw state.json | ConvertFrom-Json }
try {
  if ($state.complete) { exit }
  if ($state.runs -ge 8 -or $state.failures -ge 3) { throw 'Setup retry limit reached' }
  $state.runs++; Save
  $cfg = Get-Content -Raw inventory.json | ConvertFrom-Json
  if (!$state.ssh) {
    Set-NetFirewallProfile -Profile Domain,Private,Public -Enabled True
    if (!(Get-NetFirewallRule -Name VastSSHGuard -ErrorAction SilentlyContinue)) {
      $ports = @('22',[string]$cfg.ssh.port) | Select-Object -Unique
      New-NetFirewallRule -Name VastSSHGuard -DisplayName 'SSH setup guard' -Direction Inbound -Protocol TCP -LocalPort $ports -Action Block -Profile Any | Out-Null
    }
    if (!(Get-Service sshd -ErrorAction SilentlyContinue)) {
      $null = Run msiexec.exe @('/i',"$root\OpenSSH-Win64.msi",'/qn','/norestart') @(0,3010) 600
    }
    Stop-Service sshd -ErrorAction SilentlyContinue
    $image = [Environment]::ExpandEnvironmentVariables((Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\sshd').ImagePath)
    if ($image -notmatch '^"?(.+?sshd\.exe)"?(?:\s|$)') { throw 'Unknown sshd path' }
    $sshd = $Matches[1]
    $ssh = 'C:\ProgramData\ssh'
    New-Item -ItemType Directory -Force $ssh | Out-Null
    [IO.File]::WriteAllLines("$ssh\administrators_authorized_keys",[string[]]$cfg.ssh.authorized_keys,[Text.Encoding]::ASCII)
    Copy-Item ssh_host_ed25519_key* $ssh -Force
    foreach ($name in @('administrators_authorized_keys','ssh_host_ed25519_key')) {
      $null = Run icacls.exe @("$ssh\$name",'/inheritance:r','/grant:r','*S-1-5-18:F','*S-1-5-32-544:F')
      $null = Run icacls.exe @("$ssh\$name",'/setowner','*S-1-5-18')
    }
    @"
Port $($cfg.ssh.port)
HostKey __PROGRAMDATA__/ssh/ssh_host_ed25519_key
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
AuthenticationMethods publickey
AllowUsers vast
AuthorizedKeysFile __PROGRAMDATA__/ssh/administrators_authorized_keys
Subsystem sftp sftp-server.exe
"@ | Set-Content "$ssh\sshd_config" -Encoding ascii
    $null = Run $sshd @('-t')
    New-Item 'HKLM:\SOFTWARE\OpenSSH' -Force | Out-Null
    New-ItemProperty 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell -Value 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -PropertyType String -Force | Out-Null
    Set-Service sshd -StartupType Automatic
    Start-Service sshd
    Get-NetFirewallRule -Name VastSSH -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -Name VastSSH -DisplayName 'Vast SSH' -Direction Inbound -Protocol TCP -LocalPort ([int]$cfg.ssh.port) -Action Allow -Profile Any | Out-Null
    Get-NetFirewallRule -Name VastSSHGuard | Remove-NetFirewallRule
    $null = Run powercfg.exe @('/change','standby-timeout-ac','0')
    $state.ssh=$true; Save
  }
  for ($i=0; $i -lt 90; $i++) {
    if ((Get-Service sshd).Status -eq 'Running' -and (Get-NetTCPConnection -State Listen -LocalPort ([int]$cfg.ssh.port) -ErrorAction SilentlyContinue) -and (Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue)) { break }
    Start-Sleep 2
  }
  if ($i -eq 90) { throw 'Network/SSH unavailable' }
  if ((Get-PSDrive C).Free -lt 15GB) { throw 'GPU setup needs 15 GiB free' }
  if (!$state.driver) {
    $driver = Download nvidia.exe
    NvidiaSignature $driver
    $extractor = Download 7zr.exe
    $null = Run $extractor @('x','-y',"-o$root\driver",$driver) @(0) 900
    NvidiaSignature "$root\driver\setup.exe"
    $code = Run "$root\driver\setup.exe" @('-s','-n','Display.Driver') @(0,1)
    $service = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm'
    if ($service.Type -ne 1 -or $service.ImagePath -notmatch 'nvlddmkm\.sys') { throw 'NVIDIA driver not registered' }
    $state.driver=$true; Save
    if ($code -eq 1) { Reboot; exit }
  }
  $state | Add-Member -NotePropertyName cuda_skipped_for_prebuilt_probes -NotePropertyValue $true -Force; Save
  $smi = "$env:windir\System32\nvidia-smi.exe"
  if (!(Test-Path $smi)) { $smi = "$env:ProgramFiles\NVIDIA Corporation\NVSMI\nvidia-smi.exe" }
  $rows = @(& $smi --query-gpu=uuid,name,driver_version --format=csv,noheader)
  if ($LASTEXITCODE -ne 0 -or !$rows.Count -or @($rows | Where-Object { $_ -notmatch ',\s*581\.51\s*$' }).Count) {
    if (!$state.reboots) { Reboot; exit }
    throw 'NVIDIA GPU validation failed'
  }
  $devices = @(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
  if ($devices.Count -ne $rows.Count -or @($devices | Where-Object Status -ne OK).Count) { throw 'NVIDIA PnP devices are not all healthy' }
  $state.complete=$true; $state.error=$null; Save
  Write-Output $rows
  Write-Output 'SETUP_COMPLETE: SSH and NVIDIA 581.51 installed; CUDA toolkit omitted, prebuilt driver-API probes used'
} catch {
  $state.failures++; $state.error="$_"; Save
  Write-Error $_ -ErrorAction Continue
  exit 1
} finally { Stop-Transcript | Out-Null }
POWERSHELL
cat >"$work/payload/packages.txt" <<'PACKAGES'
nvidia.exe cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a https://developer.nvidia.com/downloads/vulkan-beta-58151-windows
cuda.exe 9244f71a248ae99a71e2974907fa16818090d48f0589a5b67035e1dbe1e05c03 https://developer.download.nvidia.com/compute/cuda/13.0.0/local_installers/cuda_13.0.0_windows.exe
7zr.exe ad4c82fadcbdf93c03b4fc440f300509c7d60c5c2f4d183e35d9d70d6957037d https://github.com/ip7z/7zip/releases/download/26.03/7zr.exe
PACKAGES
cat >"$work/payload/components.txt" <<'COMPONENTS'
nvcc crt nvvm cudart nvrtc nvrtc_dev nvjitlink nvfatbin nvptxcompiler thrust cublas cublas_dev cufft cufft_dev curand curand_dev cusolver cusolver_dev cusparse cusparse_dev npp npp_dev nvjpeg nvjpeg_dev nvml_dev opencl cuda_profiler_api cupti sanitizer nvtx cuobjdump nvdisasm nvprune cuxxfilt
COMPONENTS
chmod 700 "$work/worker.sh"
$render_only && exit 0
systemd-run --no-block --unit=vast-reinstall \
    --property=StandardOutput=journal+console --property=StandardError=journal+console \
    /bin/bash "$work/worker.sh"
echo 'Queued. Follow progress: journalctl -fu vast-reinstall'
