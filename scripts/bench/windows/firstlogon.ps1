# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# kf-firstlogon.ps1 — run ONCE, elevated, at the first logon of a fresh win_vm.sh install
# (autounattend FirstLogonCommands). It lives on the install-time unattend.iso next to the OpenSSH
# MSI and the harness's administrators_authorized_keys, and logs everything to C:\kf\firstlogon.log.
#
# Steps: the Red Hat virtio drivers (pnputil, idempotent with offlineServicing), the QEMU guest
# agent, OpenSSH server keyed to the harness, power/recovery settings that keep a headless harness
# from stalling, then C:\kf\install-done.txt and a shutdown. The shutdown runs even when a step
# fails: win_vm.sh reads the outcome back over ssh (or the guest agent) on the check boot, and a VM
# that never powers off would only look "still installing".
param([Parameter(Mandatory = $true)][string]$Src)

$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
Start-Transcript -Path C:\kf\firstlogon.log -Append | Out-Null
$failed = @()
function Step([string]$name, [scriptblock]$body) {
    Write-Output "START $name $(Get-Date -Format o)"
    try {
        & $body
        $rc = if ($LASTEXITCODE) { $LASTEXITCODE } else { 0 }
    } catch {
        Write-Output "  exception: $_"
        $rc = 1
    }
    Write-Output "EXIT $name rc=$rc $(Get-Date -Format o)"
    if ($rc -ne 0 -and $rc -ne 3010) { $script:failed += $name }
    $global:LASTEXITCODE = 0
}

$virtio = (Get-Volume | Where-Object { $_.FileSystemLabel -like 'virtio-win*' } | Select-Object -First 1).DriveLetter
Write-Output "unattend media: $Src  virtio-win: ${virtio}:"

Step 'virtio-drivers' {
    if (-not $virtio) { throw 'no virtio-win CD (label virtio-win*)' }
    foreach ($d in 'NetKVM', 'vioserial', 'viorng', 'pvpanic', 'fwcfg', 'smbus') {
        pnputil /add-driver "${virtio}:\$d\w11\amd64\*.inf" /install
        Write-Output "  pnputil $d rc=$LASTEXITCODE"
    }
    $global:LASTEXITCODE = 0
}

Step 'qemu-guest-agent' {
    $p = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @('/i', "${virtio}:\guest-agent\qemu-ga-x86_64.msi", '/qn', '/l*v', 'C:\kf\qemu-ga-msi.log')
    $global:LASTEXITCODE = $p.ExitCode
}

Step 'openssh-install' {
    $msi = Get-ChildItem "$Src\" -Filter 'OpenSSH-Win64-*.msi' | Select-Object -First 1
    if (-not $msi) { throw "no OpenSSH MSI on $Src" }
    $p = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @('/i', $msi.FullName, '/qn', '/l*v', 'C:\kf\openssh-msi.log')
    $global:LASTEXITCODE = $p.ExitCode
}

Step 'openssh-config' {
    # The first start writes C:\ProgramData\ssh\sshd_config and the host keys.
    Start-Service sshd
    Stop-Service sshd
    New-Item -ItemType Directory -Force -Path C:\ProgramData\ssh | Out-Null
    $ak = 'C:\ProgramData\ssh\administrators_authorized_keys'
    Copy-Item "$Src\administrators_authorized_keys" $ak -Force
    icacls $ak /inheritance:r /grant '*S-1-5-32-544:F' /grant 'SYSTEM:F' | Out-Null
    $cfg = 'C:\ProgramData\ssh\sshd_config'
    $lines = Get-Content $cfg | Where-Object { $_ -notmatch '^\s*#?\s*PasswordAuthentication\b' }
    Set-Content -Path $cfg -Value (@('PasswordAuthentication no') + $lines) -Encoding ascii
    New-Item -Path 'HKLM:\SOFTWARE\OpenSSH' -Force | Out-Null
    New-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell -PropertyType String -Force `
        -Value 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' | Out-Null
    Set-Service sshd -StartupType Automatic
    Start-Service sshd
    if (-not (Get-NetFirewallRule -Name kf-sshd -ErrorAction SilentlyContinue)) {
        New-NetFirewallRule -Name kf-sshd -DisplayName 'kayfabe harness sshd' -Protocol TCP -LocalPort 22 -Action Allow -Profile Any | Out-Null
    }
    Get-Service sshd | Format-List Name, Status, StartType
}

Step 'power-and-recovery' {
    powercfg /h off
    powercfg /change standby-timeout-ac 0
    powercfg /change monitor-timeout-ac 0
    # A driver bugcheck loop must not land in WinRE and stall the harness.
    bcdedit /set '{current}' recoveryenabled No
    bcdedit /set '{current}' bootstatuspolicy IgnoreAllFailures
    $global:LASTEXITCODE = 0
}

Step 'defender-exclusion' {
    # The harness's own directory only (evidence, probes); Defender stays on.
    Add-MpPreference -ExclusionPath 'C:\kf'
}

Step 'autologon-cleanup' {
    Remove-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon' -Name DefaultPassword -ErrorAction SilentlyContinue
    Set-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon' -Name AutoAdminLogon -Value '0'
}

$status = if ($failed.Count) { "FAILED: $($failed -join ',')" } else { 'OK' }
Set-Content -Path C:\kf\install-done.txt -Value "KF_FIRSTLOGON $status $(Get-Date -Format o)" -Encoding ascii
Write-Output "KF_FIRSTLOGON $status"
Stop-Transcript | Out-Null
shutdown /s /t 10 /c "kayfabe win_vm.sh: first logon done"
