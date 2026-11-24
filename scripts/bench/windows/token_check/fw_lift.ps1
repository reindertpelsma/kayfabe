# propimage_token_check firewall lift, ONLY on the throwaway overlay and ONLY after fw_detect.ps1 recorded the state of the seal.
# STATUS: LIVE, 2026-11-18 (untested in a guest). Lifts only rules it can name: the 'kf-offline' group and enabled outbound Block rules whose
# Name or DisplayName starts with kayfabe or kf-. The inbound ssh rule stays enabled (otherwise `report` and `end` could not ssh in again).
# $LiftAll (bool, set by the caller, --lift-all): ALSO disable every other enabled outbound Block rule and set the default outbound action of
# the profiles to Allow. Without it, anything else is only reported by fw_detect.ps1 and left alone.
$ErrorActionPreference = 'SilentlyContinue'
$named = @(Get-NetFirewallRule -Enabled True -Direction Outbound -Action Block | Where-Object { $_.Name -match '^(kayfabe|kf-)' -or $_.DisplayName -match '^(kayfabe|kf-)' -or $_.Group -eq 'kf-offline' })
foreach ($r in $named) { "LIFTED_RULE=" + $r.Name; $r | Disable-NetFirewallRule }
if ($LiftAll) {
  Set-NetFirewallProfile -Profile Domain,Private,Public -DefaultOutboundAction Allow
  Get-NetFirewallRule -Enabled True -Direction Outbound -Action Block | Disable-NetFirewallRule
  "LIFTED_ALL=yes"
}
Get-NetFirewallRule -Name 'kf-offline-ssh-in' | Enable-NetFirewallRule
"after: " + ((Get-NetFirewallProfile | ForEach-Object { $_.Name + '=' + $_.Enabled + '/out:' + $_.DefaultOutboundAction }) -join ' ')
"outbound block rules still enabled: " + @(Get-NetFirewallRule -Enabled True -Direction Outbound -Action Block).Count
$r = Test-NetConnection -ComputerName 1.1.1.1 -Port 443 -WarningAction SilentlyContinue
"connect 1.1.1.1:443 -> " + $r.TcpTestSucceeded
"KF_TC_DONE"
