# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# desktop_prep.ps1 -- make a win_vm.sh guest reach a signed-in desktop by itself (`win_vm.sh desktop-prep`).
#
#   1. Winlogon autologon for -User: AutoAdminLogon=1, DefaultUserName, DefaultPassword, DefaultDomainName='.',
#      and no AutoLogonCount (the unattend's AutoLogon LogonCount=1 would otherwise be spent once).
#   2. PasswordLess\Device\DevicePasswordLessBuildVersion=0. Without it Windows reset AutoAdminLogon to 0 and
#      dropped DefaultPassword at the boot after the first one (MEASURED 2026-10-08, owner-host run 58,
#      scripts/bench/windows/windows_broker.sh `autologon`; the cause is inferred, not proven).
#   3. Dwm\OverlayTestMode=5 (docs/howto/windows-guest.md, setting 3).
#   Every value is read back; the last line is `DESKTOP_PREP_DONE fail=N`.
#
# The password is ONE LINE on stdin (-PasswordFromStdin); it is never an argument, never echoed, never in the
# transcript. It ends up in the guest registry (DefaultPassword) -- that is the point of autologon -- so this is
# for a throwaway guest on an untrusted box only (owner-delegated 2026-10-31). The values apply at the NEXT boot.
param(
    [string]$User = 'kf',
    [switch]$PasswordFromStdin
)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
Start-Transcript -Path C:\kf\desktop_prep.log -Append | Out-Null
function Say([string]$s) { Write-Output "DESKTOP_PREP $s" }

if ($User -notmatch '^[A-Za-z0-9_.-]{1,20}$') { Say 'REFUSED -User is not a plain account name'; Stop-Transcript | Out-Null; exit 2 }
if (-not $PasswordFromStdin) { Say 'REFUSED -PasswordFromStdin is required (no password in arguments)'; Stop-Transcript | Out-Null; exit 2 }
$pw = [Console]::In.ReadLine()
if (-not $pw) { Say 'REFUSED no password on stdin'; Stop-Transcript | Out-Null; exit 2 }
if (-not (Get-LocalUser -Name $User -ErrorAction SilentlyContinue)) {
    Say "REFUSED no local account '$User'"; Stop-Transcript | Out-Null; exit 2
}

$script:fail = 0
function Check([string]$name, $want, $got) {
    $ok = ([string]$got -eq [string]$want)
    if (-not $ok) { $script:fail++ }
    Say ("{0}={1} want={2} {3}" -f $name, $got, $want, $(if ($ok) { 'OK' } else { 'MISMATCH' }))
}

$wl = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
Set-ItemProperty -Path $wl -Name AutoAdminLogon -Value '1' -Type String
Set-ItemProperty -Path $wl -Name DefaultUserName -Value $User -Type String
Set-ItemProperty -Path $wl -Name DefaultPassword -Value $pw -Type String
Set-ItemProperty -Path $wl -Name DefaultDomainName -Value '.' -Type String
Remove-ItemProperty -Path $wl -Name AutoLogonCount -ErrorAction SilentlyContinue

$pl = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\PasswordLess\Device'
New-Item -Path $pl -Force | Out-Null
Set-ItemProperty -Path $pl -Name DevicePasswordLessBuildVersion -Value 0 -Type DWord

$dwm = 'HKLM:\SOFTWARE\Microsoft\Windows\Dwm'
New-Item -Path $dwm -Force | Out-Null
Set-ItemProperty -Path $dwm -Name OverlayTestMode -Value 5 -Type DWord

$g = Get-ItemProperty -Path $wl
Check 'AutoAdminLogon' '1' $g.AutoAdminLogon
Check 'DefaultUserName' $User $g.DefaultUserName
Check 'DefaultDomainName' '.' $g.DefaultDomainName
Check 'DefaultPassword_matches' 'True' ($g.DefaultPassword -ceq $pw)     # a boolean only, never the value
Check 'AutoLogonCount_absent' 'True' ($null -eq $g.AutoLogonCount)
Check 'DevicePasswordLessBuildVersion' '0' (Get-ItemProperty -Path $pl).DevicePasswordLessBuildVersion
Check 'OverlayTestMode' '5' (Get-ItemProperty -Path $dwm).OverlayTestMode
Write-Output "DESKTOP_PREP_DONE fail=$script:fail"
Stop-Transcript | Out-Null
exit $(if ($script:fail) { 1 } else { 0 })
