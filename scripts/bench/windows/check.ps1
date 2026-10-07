# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# check.ps1 — the state a win_vm.sh guest must be in, one `WIN_<ITEM> PASS|FAIL|INFO <detail>` line
# per item; exit 1 if any line is FAIL. Run over ssh by `win_vm.sh check` (after the install, and
# again after the NVIDIA driver install: the owner's BitLocker ruling is re-checked then too).
#
#   -ExpectKf3   the kf3 device is attached: its display device is reported, not required absent
#   -DbMatch S   the Secure Boot db must contain the ASCII string S (the kayfabe cert's CN)
param([switch]$ExpectKf3, [string]$DbMatch = 'kayfabe GOP')

$ErrorActionPreference = 'Continue'
$script:fail = 0
function Line([string]$item, [string]$verdict, [string]$detail) {
    if ($verdict -eq 'FAIL') { $script:fail = 1 }
    Write-Output ("WIN_{0} {1} {2}" -f $item, $verdict, ($detail -replace '\s+', ' ').Trim())
}

$os = Get-CimInstance Win32_OperatingSystem
Line 'EDITION' $(if ($os.Caption -match 'Enterprise' -and $os.BuildNumber -ge 26100) { 'PASS' } else { 'FAIL' }) "$($os.Caption) build $($os.BuildNumber) $($os.Version)"

$sb = try { Confirm-SecureBootUEFI } catch { "error: $_" }
Line 'SECUREBOOT' $(if ($sb -eq $true) { 'PASS' } else { 'FAIL' }) "Confirm-SecureBootUEFI=$sb"

$db = try { [Text.Encoding]::ASCII.GetString((Get-SecureBootUEFI -Name db).Bytes) } catch { '' }
$dbHas = $db -match [regex]::Escape($DbMatch)
$dbMs = @('Windows Production PCA 2011', 'Microsoft Corporation UEFI CA 2011', 'Windows UEFI CA 2023', 'Microsoft UEFI CA 2023', 'Microsoft Option ROM UEFI CA 2023') |
    Where-Object { $db -match [regex]::Escape($_) }
Line 'DB_HAS_KAYFABE' $(if ($dbHas) { 'PASS' } else { 'FAIL' }) "db contains '$DbMatch'=$dbHas; Microsoft entries: $($dbMs -join '; ')"

$tpm = Get-Tpm
$spec = (Get-CimInstance -Namespace root/cimv2/security/microsofttpm -ClassName Win32_Tpm -ErrorAction SilentlyContinue).SpecVersion
Line 'TPM' $(if ($tpm.TpmPresent -and $tpm.TpmReady -and $spec -match '^2\.0') { 'PASS' } else { 'FAIL' }) "present=$($tpm.TpmPresent) ready=$($tpm.TpmReady) enabled=$($tpm.TpmEnabled) activated=$($tpm.TpmActivated) owned=$($tpm.TpmOwned) spec=$spec"

# The TPM's identity: the endorsement key's public part. It must be identical across guest reboots
# and QEMU restarts (the swtpm state is manufactured once and persists; owner, 2026-10-04).
$ek = try {
    $info = Get-TpmEndorsementKeyInfo -HashAlgorithm Sha256
    "PublicKeyHash=$($info.PublicKeyHash) ManufacturerCertificates=$($info.ManufacturerCertificates.Count)"
} catch { "error: $_" }
Line 'TPM_EK' $(if ($ek -match '^PublicKeyHash=[0-9A-Fa-f]{64}') { 'INFO' } else { 'FAIL' }) $ek

$bl = Get-BitLockerVolume -MountPoint C: -ErrorAction SilentlyContinue
$pde = (Get-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Control\BitLocker -Name PreventDeviceEncryption -ErrorAction SilentlyContinue).PreventDeviceEncryption
$mbde = (manage-bde -status C: 2>&1 | Select-String 'Conversion Status|Percentage Encrypted|Protection Status|Encryption Method') -join '; '
$blOk = $bl -and $bl.VolumeStatus -eq 'FullyDecrypted' -and $bl.ProtectionStatus -eq 'Off' -and $pde -eq 1
Line 'BITLOCKER' $(if ($blOk) { 'PASS' } else { 'FAIL' }) "VolumeStatus=$($bl.VolumeStatus) ProtectionStatus=$($bl.ProtectionStatus) PreventDeviceEncryption=$pde; manage-bde: $mbde"

$dg = Get-CimInstance -Namespace root\Microsoft\Windows\DeviceGuard -ClassName Win32_DeviceGuard -ErrorAction SilentlyContinue
Line 'VBS' $(if ($dg.VirtualizationBasedSecurityStatus -eq 0) { 'PASS' } else { 'FAIL' }) "VirtualizationBasedSecurityStatus=$($dg.VirtualizationBasedSecurityStatus)"

$want = 'viostor.inf', 'netkvm.inf', 'vioser.inf', 'viorng.inf', 'pvpanic.inf', 'fwcfg.inf', 'smbus.inf'
$store = ((pnputil /enum-drivers | Out-String) -split '(\r?\n){2,}') | Where-Object { $_ -match 'Red Hat' } |
    ForEach-Object { if ($_ -match 'Original Name:\s+(\S+)') { $Matches[1].ToLower() } }
$missing = $want | Where-Object { $store -notcontains $_ }
$balloon = $store | Where-Object { $_ -match 'balloon|viomem' }
$bound = (Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DriverProviderName -match 'Red Hat' }).DeviceName
Line 'VIRTIO' $(if (-not $missing -and -not $balloon) { 'PASS' } else { 'FAIL' }) "store: $($store -join ','); missing: $($missing -join ','); balloon/viomem staged: $($balloon -join ','); bound: $($bound -join '; ')"

$bad = Get-PnpDevice -PresentOnly | Where-Object { $_.Problem -and $_.Problem -ne 'CM_PROB_NONE' -and $_.Problem -ne 0 }
$badTxt = ($bad | ForEach-Object { "$($_.FriendlyName) [$($_.InstanceId)] $($_.Problem)" }) -join '; '
if ($ExpectKf3) {
    $other = $bad | Where-Object { $_.InstanceId -notlike 'PCI\VEN_10DE*' }
    Line 'PNP_CLEAN' $(if (-not $other) { 'PASS' } else { 'FAIL' }) "problem devices: $badTxt"
} else {
    Line 'PNP_CLEAN' $(if (-not $bad) { 'PASS' } else { 'FAIL' }) "problem devices: $badTxt"
}

$disp = Get-PnpDevice -PresentOnly -Class Display -ErrorAction SilentlyContinue | ForEach-Object { "$($_.FriendlyName) [$($_.InstanceId)] status=$($_.Status) problem=$($_.Problem)" }
Line 'DISPLAY' 'INFO' ($disp -join '; ')

$svc = Get-Service sshd, QEMU-GA -ErrorAction SilentlyContinue
$svcOk = ($svc | Where-Object Status -eq 'Running').Count -eq 2
Line 'SERVICES' $(if ($svcOk) { 'PASS' } else { 'FAIL' }) (($svc | ForEach-Object { "$($_.Name)=$($_.Status)" }) -join ' ')

$au = (Get-ItemProperty HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU -ErrorAction SilentlyContinue).NoAutoUpdate
$wud = (Get-ItemProperty HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate -ErrorAction SilentlyContinue).ExcludeWUDriversInQualityUpdate
Line 'WU_OFF' $(if ($au -eq 1 -and $wud -eq 1) { 'PASS' } else { 'FAIL' }) "NoAutoUpdate=$au ExcludeWUDriversInQualityUpdate=$wud wuauserv=$((Get-Service wuauserv).Status)"

$hib = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' -ErrorAction SilentlyContinue).HiberbootEnabled
$pa = (powercfg /a 2>&1 | Out-String)
Line 'NO_HIBER' $(if ($hib -eq 0 -and $pa -match 'Hibernation has not been enabled|hibernation is not available|The hiberfile type does not support|Hibernate') { 'PASS' } else { 'FAIL' }) "HiberbootEnabled=$hib"

$sac = (Get-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy -ErrorAction SilentlyContinue).VerifiedAndReputablePolicyState
Line 'SAC_OFF' $(if ($sac -eq 0) { 'PASS' } else { 'FAIL' }) "VerifiedAndReputablePolicyState=$sac"

$done = Get-Content C:\kf\install-done.txt -ErrorAction SilentlyContinue
Line 'FIRSTLOGON' $(if ($done -match 'KF_FIRSTLOGON OK') { 'PASS' } else { 'FAIL' }) "$done"

Write-Output "WIN_CHECK_DONE fail=$script:fail"
exit $script:fail
