# SPDX-License-Identifier: Apache-2.0
<#
esp_swap.ps1 -- run INSIDE a Windows 11 UEFI VM (as an administrator, over ssh or locally).

  Facts    print what decides whether the swap can work (Secure Boot, NVRAM entries, ESP size/free,
           disk layout, NIC, TPM, BitLocker, edition/activation). Writes nothing.
  Swap     back up the Windows boot files on the ESP to \EFI\kf-backup, then replace BOTH
           \EFI\Microsoft\Boot\bootmgfw.efi (what the NVRAM "Windows Boot Manager" entry starts) and
           \EFI\BOOT\BOOTX64.EFI (the removable-media fallback) with the UKI. Refuses when Secure
           Boot is on (an unsigned UKI would not start) or when the UKI hash is not the expected one.
  Restore  put the originals back (the same thing esp_restore.sh does from Alpine) and verify.

  powershell -File esp_swap.ps1 -Mode Facts
  powershell -File esp_swap.ps1 -Mode Swap    -Uki C:\kf\kf-alpine.efi -ExpectSha256 <hex>
  powershell -File esp_swap.ps1 -Mode Restore

Output lines start with ESP_ so a caller can grep them. Nothing here touches the BCD store or the
NVRAM; the only writes are inside the ESP (\EFI\kf-backup, the two boot paths).
#>
param(
  [Parameter(Mandatory = $true)][ValidateSet('Facts', 'Swap', 'Restore')][string]$Mode,
  [string]$Uki = 'C:\kf\kf-alpine.efi',
  [string]$ExpectSha256 = '',
  [switch]$IgnoreSecureBoot
)
$ErrorActionPreference = 'Stop'
function Say($m) { Write-Output ("ESP_" + $m) }
function Sha($p) { try { (Get-FileHash -Algorithm SHA256 -LiteralPath $p -ErrorAction Stop).Hash.ToLower() } catch { 'unreadable(in-use)' } }

function Get-SecureBootState {
  try { if (Confirm-SecureBootUEFI) { 'on' } else { 'off' } }
  catch { 'unsupported-or-bios:' + $_.Exception.Message.Trim() }
}

function Mount-Esp {
  # mount the ESP on a free letter with mountvol /S; return the letter with colon
  foreach ($l in 'S', 'T', 'U', 'V', 'W') {
    if (-not (Test-Path "${l}:\")) {
      $o = & mountvol "${l}:" /S 2>&1
      if ($LASTEXITCODE -ne 0) { throw "mountvol ${l}: /S failed: $o" }
      Start-Sleep -Milliseconds 500
      if (Test-Path "${l}:\EFI") { return "${l}:" }
      & mountvol "${l}:" /D | Out-Null
    }
  }
  throw 'could not mount the ESP on S: to W:'
}

function Dismount-Esp($d) { & mountvol $d /D | Out-Null }
function Esp-Free($d) { (New-Object IO.DriveInfo($d.Substring(0, 1))).AvailableFreeSpace }
function Esp-Size($d) { (New-Object IO.DriveInfo($d.Substring(0, 1))).TotalSize }

if ($Mode -eq 'Facts') {
  Say ("SECUREBOOT " + (Get-SecureBootState))
  Say ("OS " + ((Get-CimInstance Win32_OperatingSystem | ForEach-Object { "$($_.Caption) build $($_.BuildNumber) arch $($_.OSArchitecture)" })))
  try { Say ("EDITION " + (Get-WindowsEdition -Online).Edition) } catch { Say "EDITION unknown: $($_.Exception.Message)" }
  Get-CimInstance SoftwareLicensingProduct -Filter "Name like 'Windows%' and PartialProductKey is not null" |
    ForEach-Object { Say ("LICENSE name=" + $_.Name + " status=" + $_.LicenseStatus + " (0 unlicensed,1 licensed,5 notification) gracemin=" + $_.GracePeriodRemaining + " channel=" + $_.ProductKeyChannel) }
  Say 'FIRMWARE_ENTRIES_BEGIN'
  & bcdedit /enum firmware 2>&1 | ForEach-Object { Say ("  " + $_) }
  Say 'FIRMWARE_ENTRIES_END'
  & bcdedit /enum '{bootmgr}' 2>&1 | ForEach-Object { Say ("BOOTMGR " + $_) }
  Say 'DISKS_BEGIN'
  Get-Disk | ForEach-Object { Say ("DISK n=$($_.Number) size=$($_.Size) style=$($_.PartitionStyle) bus=$($_.BusType) model=$($_.FriendlyName) boot=$($_.IsBoot) sys=$($_.IsSystem)") }
  Get-Partition | ForEach-Object { Say ("PART disk=$($_.DiskNumber) n=$($_.PartitionNumber) type=$($_.Type) size=$($_.Size) offset=$($_.Offset) letter=$($_.DriveLetter) gpt=$($_.GptType)") }
  Get-Volume | ForEach-Object { Say ("VOL letter=$($_.DriveLetter) fs=$($_.FileSystem) label=$($_.FileSystemLabel) size=$($_.Size) free=$($_.SizeRemaining)") }
  Say 'DISKS_END'
  $esp = Get-Partition | Where-Object { $_.GptType -eq '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}' } | Select-Object -First 1
  if ($esp) {
    Say ("ESP disk=$($esp.DiskNumber) part=$($esp.PartitionNumber) size=$($esp.Size)")
    $d = Mount-Esp
    try {
      Say ("ESP_FS fs=$((New-Object IO.DriveInfo($d.Substring(0, 1))).DriveFormat) size=$(Esp-Size $d) free=$(Esp-Free $d)")
      Get-ChildItem "$d\EFI" -Recurse -File | ForEach-Object { Say ("ESPFILE " + $_.FullName.Substring(2) + " bytes=" + $_.Length + " sha256=" + (Sha $_.FullName)) }
    } finally { Dismount-Esp $d }
  } else { Say 'ESP not found' }
  Get-NetAdapter | ForEach-Object { Say ("NIC name=$($_.Name) desc=$($_.InterfaceDescription) mac=$($_.MacAddress) status=$($_.Status) speed=$($_.LinkSpeed)") }
  Get-NetIPConfiguration | ForEach-Object { Say ("IPCONF if=$($_.InterfaceAlias) ipv4=$($_.IPv4Address.IPAddress)/$($_.IPv4Address.PrefixLength) gw=$($_.IPv4DefaultGateway.NextHop) dns=$(($_.DNSServer.ServerAddresses) -join ',') dhcp=$((Get-NetIPInterface -InterfaceAlias $_.InterfaceAlias -AddressFamily IPv4).Dhcp)") }
  try { $t = Get-Tpm; Say ("TPM present=$($t.TpmPresent) ready=$($t.TpmReady) enabled=$($t.TpmEnabled) activated=$($t.TpmActivated)") } catch { Say "TPM query failed: $($_.Exception.Message)" }
  try { Get-BitLockerVolume | ForEach-Object { Say ("BITLOCKER mount=$($_.MountPoint) status=$($_.VolumeStatus) protection=$($_.ProtectionStatus)") } } catch { Say "BITLOCKER query failed: $($_.Exception.Message)" }
  & manage-bde -status C: 2>&1 | Select-Object -First 14 | ForEach-Object { Say ("MANAGEBDE " + $_) }
  Say ("HIBERNATE_FASTSTARTUP HiberbootEnabled=" + (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' -Name HiberbootEnabled -ErrorAction SilentlyContinue).HiberbootEnabled)
  & powercfg /a 2>&1 | Select-Object -First 12 | ForEach-Object { Say ("POWERCFG " + $_) }
  Say ("VIRTIO_DRIVERS " + ((Get-WindowsDriver -Online -ErrorAction SilentlyContinue | Where-Object { $_.OriginalFileName -match 'vio|viostor|netkvm|virtio' } | ForEach-Object { $_.Driver }) -join ','))
  Say 'FACTS_DONE'
  exit 0
}

# ---- Swap and Restore touch the ESP
$d = Mount-Esp
try {
  $boot = "$d\EFI\Microsoft\Boot\bootmgfw.efi"; $fallback = "$d\EFI\BOOT\BOOTX64.EFI"; $bk = "$d\EFI\kf-backup"
  if ($Mode -eq 'Swap') {
    $sb = Get-SecureBootState
    if ($sb -eq 'on' -and -not $IgnoreSecureBoot) { Say "REFUSED secure boot is on (an unsigned UKI would not start)"; exit 2 }
    if (-not (Test-Path -LiteralPath $Uki)) { Say "REFUSED no UKI at $Uki"; exit 2 }
    $us = Sha $Uki
    if ($ExpectSha256 -and $us -ne $ExpectSha256.ToLower()) { Say "REFUSED UKI sha256 $us is not the expected $ExpectSha256"; exit 2 }
    if (-not (Test-Path -LiteralPath $boot)) { Say "REFUSED $boot does not exist (not a Windows ESP)"; exit 2 }
    $uk = (Get-Item -LiteralPath $Uki).Length
    $free = Esp-Free $d
    $oldBoot = (Get-Item -LiteralPath $boot).Length
    $hasFb = Test-Path -LiteralPath $fallback
    $oldFb = if ($hasFb) { (Get-Item -LiteralPath $fallback).Length } else { 0 }
    # new data = UKI at the NVRAM path (replacing the old file) + UKI at the fallback path + the backups (the two old files and BCD)
    $need = ($uk - $oldBoot) + ($uk - $oldFb) + $oldBoot + $oldFb + 2MB + 4MB
    if ($free -lt $need) { Say "REFUSED ESP free space $free is below the $need bytes needed for two UKI copies of $uk bytes plus backups"; exit 2 }
    New-Item -ItemType Directory -Force -Path $bk | Out-Null
    if (-not (Test-Path "$bk\bootmgfw.efi")) {   # never overwrite a backup with the UKI on a second run
      if ((Sha $boot) -eq $us) { Say 'REFUSED bootmgfw.efi already is the UKI and no backup exists'; exit 2 }
      Copy-Item -LiteralPath $boot -Destination "$bk\bootmgfw.efi"
      if (Test-Path -LiteralPath $fallback) { Copy-Item -LiteralPath $fallback -Destination "$bk\BOOTX64.EFI" }
      else { Set-Content -Path "$bk\BOOTX64.EFI.absent" -Value 'EFI\BOOT\BOOTX64.EFI did not exist before the swap' }
      New-Item -ItemType Directory -Force -Path "$bk\Boot" | Out-Null   # the BCD store: this script never edits it, the copy is a belt
      & bcdedit /export "$bk\Boot\BCD.export" | Out-Null   # the live BCD file is locked by the system: export it
    }
    # network settings for the UKI when DHCP fails there (it reads this file from the ESP)
    $c = Get-NetIPConfiguration | Where-Object { $_.IPv4DefaultGateway } | Select-Object -First 1
    if ($c) {
      $mac = ((Get-NetAdapter -InterfaceAlias $c.InterfaceAlias).MacAddress -replace '-', ':').ToLower()
      $nc = "NET_IF_MAC=$mac`nNET_IP=$($c.IPv4Address.IPAddress)`nNET_PREFIX=$($c.IPv4Address.PrefixLength)`nNET_GW=$($c.IPv4DefaultGateway.NextHop)`nNET_DNS=$(($c.DNSServer.ServerAddresses | Select-Object -First 1))`n"
      [IO.File]::WriteAllText("$bk\net.conf", $nc.Replace("`r", ''))
    }
    & bcdedit /enum firmware 2>&1 | Set-Content "$bk\firmware_entries_before.txt"
    Get-ChildItem $bk -File -Recurse | ForEach-Object { "$((Sha $_.FullName))  $($_.FullName.Substring($bk.Length + 1))" } | Set-Content "$bk\SHA256SUMS.txt"
    New-Item -ItemType Directory -Force -Path "$d\EFI\BOOT" | Out-Null
    Copy-Item -LiteralPath $Uki -Destination $boot -Force
    Copy-Item -LiteralPath $Uki -Destination $fallback -Force
    $ok = ((Sha $boot) -eq $us) -and ((Sha $fallback) -eq $us)
    $names = (Get-ChildItem $bk -File | ForEach-Object { $_.Name }) -join ','
    Say "SWAP backup_files=$names"
    $fa = Esp-Free $d
    Say "SWAP_DONE ok=$ok uki_sha256=$us bytes=$uk esp_free_after=$fa"
    if (-not $ok) { exit 1 }
  } else {
    if (-not (Test-Path "$bk\bootmgfw.efi")) { Say "REFUSED no backup in $bk"; exit 2 }
    Copy-Item -LiteralPath "$bk\bootmgfw.efi" -Destination $boot -Force
    $ok = (Sha $boot) -eq (Sha "$bk\bootmgfw.efi")
    if (Test-Path "$bk\BOOTX64.EFI") { Copy-Item -LiteralPath "$bk\BOOTX64.EFI" -Destination $fallback -Force; $ok = $ok -and ((Sha $fallback) -eq (Sha "$bk\BOOTX64.EFI")) }
    elseif (Test-Path "$bk\BOOTX64.EFI.absent") { Remove-Item -LiteralPath $fallback -Force -ErrorAction SilentlyContinue }
    Say "RESTORE_DONE ok=$ok bootmgfw_sha256=$(Sha $boot)"
    if (-not $ok) { exit 1 }
  }
} finally { Dismount-Esp $d }
