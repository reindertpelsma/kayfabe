# propimage_token_check firewall DETECT (read-only). STATUS: LIVE, 2026-11-18 (untested in a guest). Runs BEFORE any lift; its result is the
# 'firewall in seal: yes/no' line. Prints counts, rule names that start with kayfabe or kf-, and two outbound TCP 443 probes.
$ErrorActionPreference = 'SilentlyContinue'
"FW_PROFILES=" + ((Get-NetFirewallProfile | ForEach-Object { $_.Name + '=' + $_.Enabled + '/out:' + $_.DefaultOutboundAction + '/in:' + $_.DefaultInboundAction }) -join ' ')
"FW_KF_OFFLINE_GROUP_RULES=" + @(Get-NetFirewallRule -Group 'kf-offline').Count
"FW_KAYFABE_BLOCK_RULES_ENABLED=" + @(Get-NetFirewallRule -Enabled True -Direction Outbound -Action Block | Where-Object { $_.Name -match '^(kayfabe|kf-)' -or $_.DisplayName -match '^(kayfabe|kf-)' }).Count
"FW_OTHER_OUTBOUND_BLOCK_RULES_ENABLED=" + @(Get-NetFirewallRule -Enabled True -Direction Outbound -Action Block | Where-Object { $_.Name -notmatch '^(kayfabe|kf-)' -and $_.DisplayName -notmatch '^(kayfabe|kf-)' }).Count
"FW_OUTBOUND_DEFAULT_BLOCK_PROFILES=" + @(Get-NetFirewallProfile | Where-Object { $_.Enabled -eq 'True' -and $_.DefaultOutboundAction -eq 'Block' }).Count
foreach ($r in @(Get-NetFirewallRule -Group 'kf-offline')) { "FW_KF_OFFLINE_RULE=" + $r.Name + ' enabled=' + $r.Enabled + ' dir=' + $r.Direction + ' action=' + $r.Action }
foreach ($r in @(Get-NetFirewallRule -Enabled True -Direction Outbound -Action Block | Where-Object { $_.Name -match '^(kayfabe|kf-)' -or $_.DisplayName -match '^(kayfabe|kf-)' })) { "FW_KAYFABE_BLOCK_RULE=" + $r.Name + ' enabled=' + $r.Enabled }
foreach ($h in @('1.1.1.1', '9.9.9.9')) { $res = 'error'; try { $c = New-Object Net.Sockets.TcpClient; $a = $c.BeginConnect($h, 443, $null, $null); if ($a.AsyncWaitHandle.WaitOne(5000) -and $c.Connected) { $res = 'OPEN' } else { $res = 'FAILED(timeout-or-blocked)' }; $c.Close() } catch { $res = 'FAILED(refused-or-error)' }; "FW_SEAL_TCP443_" + $h + "=" + $res }
"GUEST_UTC=" + (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
"KF_TC_DONE"
