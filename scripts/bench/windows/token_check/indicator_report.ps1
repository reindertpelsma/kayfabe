# propimage_token_check INDICATOR REPORT: read-only, prints no secrets and no e-mail addresses (only booleans, counts, sizes, dates, 'present').
# STATUS: LIVE, 2026-11-18 (GPU-free tool; the script itself is untested in a guest). Image-agnostic: it names no image, only vendor identity folders.
# Windows PowerShell 5.1, ASCII only. $Hosts (array of hostnames) is defined by the caller. Never reads token contents; the only content reads
# are: Steam loginusers.vdf/config.vdf (counted, never printed), Autodesk LoginState.xml (allow-listed true/false/0/1 attribute values),
# Edge/Chrome Preferences (boolean 'has account_info', never printed). Presence is a heuristic: the operator's UI test is the authority.
$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'
$SUM = [ordered]@{}
function S($t) { $t = [string]$t; $t = $t -replace '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}', '<account>'; $t = $t -replace 'eyJ[A-Za-z0-9_\-\.]{10,}', '<jwt>'; $t = [regex]::Replace($t, '(bytes=\d+)|\d{9,}', { param($m) if ($m.Groups[1].Success) { $m.Value } else { '<n>' } }); $t }
function P($k, $v) { Write-Output ("{0}={1}" -f $k, (S $v)) }
function H($t) { Write-Output ''; Write-Output ('## ' + $t) }
function Yn($b) { if ($b) { 'yes' } else { 'no' } }
function Sm($k, $v) { $SUM[$k] = $v }
$PDATA = $env:ProgramData
function Sh($p) { $p = [string]$p; $p = $p -replace '^C:\\Users\\[^\\]+\\', '~\'; $p.Replace($PDATA, '%ProgramData%') }
function InfoS($path) {
  if (Test-Path -LiteralPath $path) {
    $i = Get-Item -LiteralPath $path -Force
    if ($i.PSIsContainer) {
      $n = @(Get-ChildItem -LiteralPath $path -Force -File -Recurse -Depth 3 -ErrorAction SilentlyContinue)
      $sz = ($n | Measure-Object Length -Sum).Sum
      return ('dir files=' + $n.Count + ' bytes=' + [int64]$sz + ' mtime=' + $i.LastWriteTimeUtc.ToString('yyyy-MM-dd'))
    }
    return ('file bytes=' + $i.Length + ' mtime=' + $i.LastWriteTimeUtc.ToString('yyyy-MM-dd'))
  }
  return $null
}
function Msk($n) { ([string]$n) -replace '\d{6,}', '<n>' }
# a log line, masked: accounts, ids, long tokens and every run of 5+ digits are replaced; only the outcome words survive
function Mskl($l, $n) { $l = [string]$l; $l = $l -replace '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}', '<account>'; $l = $l -replace '\[U:\d+:\d+(:\d+)?\]', '[U:<id>]'; $l = $l -replace '[A-Za-z0-9_\-\.=+/]{32,}', '<long>'; $l = $l -replace '\d{5,}', '<n>'; if ($l.Length -gt $n) { $l = $l.Substring(0, $n) }; $l }
function KB($path, $keys) {
  try {
    $fi = Get-Item -LiteralPath $path -Force
    if ($fi.Length -gt 4194304) { return 'too large to scan' }
    $b = [IO.File]::ReadAllBytes($path)
    $t1 = [Text.Encoding]::GetEncoding(28591).GetString($b)
    $t2 = [Text.Encoding]::Unicode.GetString($b)
    return (($keys | ForEach-Object { $_ + ':' + $(if (($t1.IndexOf($_, [StringComparison]::OrdinalIgnoreCase) -ge 0) -or ($t2.IndexOf($_, [StringComparison]::OrdinalIgnoreCase) -ge 0)) { 'yes' } else { 'no' }) }) -join ' ')
  } catch { return 'unreadable' }
}
function ListTop($label, $dir) {
  if (-not (Test-Path -LiteralPath $dir)) { P $label 'absent'; return }
  $items = @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue | Sort-Object LastWriteTimeUtc -Descending)
  P ($label + '.entries') $items.Count
  foreach ($i in ($items | Select-Object -First 60)) {
    if ($i.PSIsContainer) { P ($label + '\' + (Msk $i.Name)) ('dir mtime_utc=' + $i.LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm')) }
    else { P ($label + '\' + (Msk $i.Name)) ('file bytes=' + $i.Length + ' mtime_utc=' + $i.LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm')) }
  }
}
function VdfBool($label, $path) {
  if (Test-Path -LiteralPath $path) { $i = Get-Item -LiteralPath $path -Force; P $label ('bytes=' + $i.Length + ' mtime_utc=' + $i.LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm') + ' contains[' + (KB $path @('ConnectCache', 'Accounts', 'RememberPassword')) + ']') }
  else { P $label 'absent' }
}
function Info($label, $path) { $s = InfoS $path; if ($s) { P $label $s } else { P $label 'absent' } }
$profs = @(Get-ChildItem 'C:\Users' -Directory -Force | Where-Object { @('Public', 'Default', 'Default User', 'All Users') -notcontains $_.Name })
$PR = @(); $pi = 0
foreach ($d in $profs) { $pi++; $PR += [pscustomobject]@{ Tag = ('U' + $pi); Cur = ($d.Name -eq $env:USERNAME); Root = $d.FullName; LA = (Join-Path $d.FullName 'AppData\Local'); AD = (Join-Path $d.FullName 'AppData\Roaming') } }
$UN = @(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' | Where-Object { $_.DisplayName })
$CT = @(cmdkey /list | Where-Object { $_ -match 'Target:' })
$RXNAME = '(?i)token|(^|[^a-z])auth([^o]|$)|credential|\.vdf$|ssfn|loginstate|loginusers|oauth|\.ims$'

function Fam($name, $la, $ad, $pd, $rx, $appRx, $credRx) {
  H $name
  $paths = @()
  foreach ($u in $PR) { foreach ($r in $la) { $paths += (Join-Path $u.LA $r) }; foreach ($r in $ad) { $paths += (Join-Path $u.AD $r) } }
  foreach ($r in $pd) { $paths += (Join-Path $PDATA $r) }
  $present = 0; $hits = @()
  foreach ($p in $paths) {
    $is = InfoS $p
    if ($is) { $present++; P ('dir ' + (Sh $p)) $is; $hits += @(Get-ChildItem -LiteralPath $p -Recurse -Force -File -Depth 4 -ErrorAction SilentlyContinue | Where-Object { $_.Name -match $rx }) }
  }
  $hits = @($hits | Sort-Object FullName -Unique)
  P 'vendor_dirs_present' ('{0} of {1} candidate paths' -f $present, $paths.Count)
  P 'name_pattern_hits' $hits.Count
  foreach ($h in ($hits | Select-Object -First 20)) { P 'hit' ('{0} bytes={1} mtime={2}' -f (Sh $h.FullName), $h.Length, $h.LastWriteTimeUtc.ToString('yyyy-MM-dd')) }
  $apps = @($UN | Where-Object { $_.DisplayName -match $appRx -or $_.Publisher -match $appRx } | ForEach-Object { $_.DisplayName } | Sort-Object -Unique)
  P 'installed_apps' $apps.Count
  foreach ($a in ($apps | Select-Object -First 15)) { P 'app' $a }
  $cc = @($CT | Where-Object { $_ -match $credRx }).Count
  P 'credential_manager_targets_matching' $cc
  Sm (($name -replace '[^A-Za-z0-9]', '_') + '.dirs_present') $present
  Sm (($name -replace '[^A-Za-z0-9]', '_') + '.name_hits') $hits.Count
  Sm (($name -replace '[^A-Za-z0-9]', '_') + '.cred_targets') $cc
}

H 'Header'
P 'utc_now' ((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))
P 'os' ((Get-CimInstance Win32_OperatingSystem).Caption + ' build ' + [Environment]::OSVersion.Version.Build)
P 'profiles_found' $PR.Count
foreach ($u in $PR) { P ('profile ' + $u.Tag) ('current_ssh_user=' + (Yn $u.Cur)) }
P 'credential_manager_targets_total' $CT.Count

H 'Network and firewall'
P 'firewall_profiles' ((Get-NetFirewallProfile | ForEach-Object { $_.Name + '=' + $_.Enabled + '/out:' + $_.DefaultOutboundAction }) -join ' ')
$ob = @(Get-NetFirewallRule -Enabled True -Direction Outbound -Action Block)
P 'enabled_outbound_block_rules' $ob.Count
foreach ($r in ($ob | Select-Object -First 10)) { P 'outbound_block_rule' $r.Name }
P 'kf_offline_group_rules_enabled' (@(Get-NetFirewallRule -Group 'kf-offline' -Enabled True).Count)
P 'hosts_file_active_lines' (@(Get-Content "$env:SystemRoot\System32\drivers\etc\hosts" | Where-Object { $_ -match '^\s*[0-9a-fA-F:.]+\s+\S' }).Count)
P 'winhttp_proxy' ((netsh winhttp show proxy) -join ' ' -replace '\s+', ' ')
P 'ie_proxy_enabled' ((Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings').ProxyEnable)
function Tcp($h) {
  try { $ips = [Net.Dns]::GetHostAddresses($h); $d = 'dns=ok(' + $ips.Count + ')' } catch { return 'dns=FAIL tcp443=skipped' }
  $res = 'error'
  try { $c = New-Object Net.Sockets.TcpClient; $a = $c.BeginConnect($h, 443, $null, $null); if ($a.AsyncWaitHandle.WaitOne(6000) -and $c.Connected) { $res = 'OPEN' } else { $res = 'timeout' }; $c.Close() } catch { $res = 'refused-or-error' }
  return ($d + ' tcp443=' + $res)
}
$reach = 0
foreach ($h in $Hosts) { $t = Tcp $h; P ('reach ' + $h) $t; if ($t -match 'tcp443=OPEN') { $reach++ } }
P 'reach_control 1.1.1.1' ((Tcp '1.1.1.1'))
Sm 'online.vendor_hosts_open' ('{0} of {1}' -f $reach, @($Hosts).Count)

H 'Windows and Microsoft account'
P 'local_users_enabled' (@(Get-LocalUser | Where-Object { $_.Enabled }).Count)
P 'local_users_microsoft_account' (@(Get-LocalUser | Where-Object { $_.PrincipalSource -eq 'MicrosoftAccount' }).Count)
$ds = (dsregcmd /status) -join "`n"
foreach ($k in @('AzureAdJoined', 'WorkplaceJoined', 'DomainJoined', 'NgcSet')) { $m = [regex]::Match($ds, $k + '\s*:\s*(\w+)'); P ('dsreg.' + $k) $(if ($m.Success) { $m.Groups[1].Value } else { 'unknown' }) }
P 'identitycrl_stored_identities' (@(Get-ChildItem 'HKCU:\Software\Microsoft\IdentityCRL\StoredIdentities').Count)
P 'onedrive_accounts_keys' (@(Get-ChildItem 'HKCU:\Software\Microsoft\OneDrive\Accounts').Count)

H 'Steam'
$steams = @()
foreach ($c in @((Get-ItemProperty 'HKCU:\Software\Valve\Steam').SteamPath, (Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Valve\Steam').InstallPath, 'C:\Program Files (x86)\Steam')) { if ($c) { $c = ([string]$c).Replace('/', '\'); if ((Test-Path -LiteralPath $c) -and ($steams -notcontains $c)) { $steams += $c } } }
P 'steam_install_dirs_found' $steams.Count
$sk = Get-ItemProperty 'HKCU:\Software\Valve\Steam'
P 'hkcu_RememberPassword' $sk.RememberPassword
P 'hkcu_AutoLoginUser_set' (Yn ($sk.AutoLoginUser))
P 'hkcu_ActiveProcess_ActiveUser_nonzero' (Yn ((Get-ItemProperty 'HKCU:\Software\Valve\Steam\ActiveProcess').ActiveUser))
Sm 'steam.hkcu_RememberPassword' $sk.RememberPassword
Sm 'steam.autologin_user_set' (Yn ($sk.AutoLoginUser))
$sUsers = 0; $sSsfn = 0; $sCache = 0
foreach ($sd in $steams) {
  $lu = Join-Path $sd 'config\loginusers.vdf'
  if (Test-Path -LiteralPath $lu) {
    $t = Get-Content -LiteralPath $lu -Raw
    $n = ([regex]::Matches($t, '(?m)^\s*"7656\d{13}"\s*$')).Count
    $sUsers += $n
    P 'loginusers.vdf' ('present users=' + $n + ' MostRecent=' + ([regex]::Matches($t, '"MostRecent"\s+"1"')).Count + ' WantsOfflineMode=' + ([regex]::Matches($t, '"WantsOfflineMode"\s+"1"')).Count + ' RememberPassword=' + ([regex]::Matches($t, '"RememberPassword"\s+"1"')).Count + ' AllowAutoLogin=' + ([regex]::Matches($t, '"AllowAutoLogin"\s+"1"')).Count)
    $ts = @([regex]::Matches($t, '"Timestamp"\s+"(\d+)"') | ForEach-Object { [int64]$_.Groups[1].Value })
    P 'loginusers.vdf_mtime_utc' ((Get-Item -LiteralPath $lu).LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm'))
    if ($ts.Count) { P 'loginusers.newest_login_timestamp_utc' (([DateTime]'1970-01-01').AddSeconds(($ts | Measure-Object -Maximum).Maximum).ToString('yyyy-MM-dd HH:mm')) }
    if ($ts.Count) { P 'loginusers.newest_login_age_days' ([int][math]::Floor((((Get-Date).ToUniversalTime() - [DateTime]'1970-01-01').TotalSeconds - ($ts | Measure-Object -Maximum).Maximum) / 86400)) }
  } else { P 'loginusers.vdf' 'absent' }
  $cv = Join-Path $sd 'config\config.vdf'
  if (Test-Path -LiteralPath $cv) {
    $c = Get-Content -LiteralPath $cv -Raw
    $m = [regex]::Match($c, '"ConnectCache"\s*\{([^}]*)\}')
    $cn = if ($m.Success) { ([regex]::Matches($m.Groups[1].Value, '"[0-9a-fA-F]{6,}"\s+"[^"]+"')).Count } else { 0 }
    $sCache += $cn
    P 'config.vdf_mtime_utc' ((Get-Item -LiteralPath $cv).LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm'))
    P 'config.vdf' ('present bytes=' + (Get-Item -LiteralPath $cv).Length + ' ConnectCache_block=' + (Yn $m.Success) + ' ConnectCache_entries=' + $cn + ' Accounts_block=' + (Yn ($c -match '"Accounts"')))
  } else { P 'config.vdf' 'absent' }
  $ss = @(Get-ChildItem -LiteralPath $sd -Filter 'ssfn*' -File -Force)
  $sSsfn += $ss.Count
  $cl = Join-Path $sd 'logs\connection_log.txt'
  if (Test-Path -LiteralPath $cl) {
    $ci = Get-Item -LiteralPath $cl -Force
    P 'connection_log' ('present bytes=' + $ci.Length + ' mtime_utc=' + $ci.LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm'))
    $cl_lines = @(Get-Content -LiteralPath $cl -Tail 4000)
    $cl_out = @($cl_lines | Where-Object { $_ -match '(?i)access ?denied|invalid ?password|expired|logon ?denied|rate ?limit|no ?connection|logged ?on|logon ?response|service ?unavailable|timeout' })
    $ad_n = @($cl_out | Where-Object { $_ -match '(?i)access ?denied' }).Count
    P 'connection_log.access_denied_lines' $ad_n
    P 'connection_log.outcome_lines' $cl_out.Count
    Sm 'steam.connection_log_access_denied' $ad_n
    foreach ($l in ($cl_out | Select-Object -Last 8)) { P 'connection_log.last' (Mskl $l 160) }
  } else { P 'connection_log' 'absent' }
  P 'ssfn_files' ($ss.Count.ToString() + $(if ($ss.Count) { ' sizes=' + (($ss | ForEach-Object { $_.Length }) -join ',') } else { '' }))
  P 'userdata_account_dirs' (@(Get-ChildItem -LiteralPath (Join-Path $sd 'userdata') -Directory -Force)).Count
  $am = @(Get-ChildItem -LiteralPath (Join-Path $sd 'steamapps') -Filter 'appmanifest_*.acf' -File)
  P 'installed_game_manifests' $am.Count
  foreach ($a in $am) { $g = [regex]::Match((Get-Content -LiteralPath $a.FullName -Raw), '"installdir"\s+"([^"]+)"'); if ($g.Success) { P 'game' $g.Groups[1].Value } }
  VdfBool 'steam config.vdf (string presence only)' (Join-Path $sd 'config\config.vdf')
  VdfBool 'steam loginusers.vdf (string presence only)' (Join-Path $sd 'config\loginusers.vdf')
  ListTop 'steam_dir config' (Join-Path $sd 'config')
  ListTop 'steam_dir userdata' (Join-Path $sd 'userdata')
  foreach ($f in @(Get-ChildItem -LiteralPath $sd -Force -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '(?i)ssfn|\.vdf$|\.crash$|\.token|login' })) { P ('steam_dir top-level ' + (Msk $f.Name)) ('file bytes=' + $f.Length + ' mtime_utc=' + $f.LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm')) }
  foreach ($u in $PR) {
    VdfBool ('steam local.vdf ' + $u.Tag + ' (string presence only)') (Join-Path $u.LA 'Steam\local.vdf')
    ListTop ('steam_localappdata ' + $u.Tag) (Join-Path $u.LA 'Steam')
  }
}
Sm 'steam.loginusers_users' $sUsers
Sm 'steam.ssfn_files' $sSsfn
Sm 'steam.connectcache_entries' $sCache
P 'steam_credential_manager_targets' (@($CT | Where-Object { $_ -match '(?i)steam' }).Count)

Fam 'Adobe' @('Adobe\OOBE', 'Adobe\Creative Cloud', 'Adobe\IMS', 'Adobe\AdobeIdentity', 'Adobe') @('Adobe\OOBE', 'Adobe\Creative Cloud Libraries', 'Adobe\AdobeIdentity', 'Adobe') @('Adobe\OOBE', 'Adobe\SLStore', 'Adobe\SLCache', 'Adobe\APFXAS', 'Adobe\Adobe Desktop Service') '(?i)token|(^|[^a-z])auth([^o]|$)|credential|identity|\bims\b|apfx|opm\.db|oobe|login|signin|sentinel|\.vdf$' '(?i)adobe' '(?i)adobe'
Sm 'Adobe.cc_desktop_installed' (Yn (Test-Path -LiteralPath 'C:\Program Files (x86)\Adobe\Adobe Creative Cloud\ACC\Creative Cloud.exe'))
P 'creative_cloud_desktop_exe' (Yn (Test-Path -LiteralPath 'C:\Program Files (x86)\Adobe\Adobe Creative Cloud\ACC\Creative Cloud.exe'))
P 'hkcu_software_adobe_subkeys' ((@(Get-ChildItem 'HKCU:\Software\Adobe' | ForEach-Object { $_.PSChildName }) | Select-Object -First 25) -join ';')

Fam 'Autodesk' @('Autodesk', 'Autodesk\Web Services', 'Autodesk\Identity Services') @('Autodesk', 'Autodesk\Web Services') @('Autodesk', 'Autodesk\ODIS', 'Autodesk\AdskLicensingService', 'Autodesk\CLM') '(?i)token|(^|[^a-z])auth([^o]|$)|credential|identity|loginstate|licen|\.json$|adsk|ims' '(?i)autodesk' '(?i)autodesk|adsk'
foreach ($u in $PR) {
  $ls = Join-Path $u.LA 'Autodesk\Web Services\LoginState.xml'
  if (Test-Path -LiteralPath $ls) {
    $x = Get-Content -LiteralPath $ls -Raw
    P ('LoginState.xml ' + $u.Tag) ('present bytes=' + (Get-Item -LiteralPath $ls).Length)
    foreach ($m in [regex]::Matches($x, '(?i)\b([A-Za-z_]*(?:LoggedIn|LoginStatus|IsSignedIn|SignedIn|Status))\s*=\s*"([^"]*)"')) { $v = $m.Groups[2].Value; if ($v -match '^(?i:true|false|0|1)$') { P ('LoginState.attr.' + $m.Groups[1].Value) $v; Sm ('Autodesk.LoginState.' + $m.Groups[1].Value) $v } }
  } else { P ('LoginState.xml ' + $u.Tag) 'absent' }
}
Fam 'Esri ArcGIS Pro' @('ESRI', 'ESRI\ArcGISPro', 'ArcGISPro') @('ESRI', 'ESRI\ArcGISPro', 'ESRI\ArcGISPro\Portals') @('ESRI', 'ESRI\ArcGISPro') '(?i)portal|signin|sign-in|token|credential|login|named|\.json$|licen' '(?i)arcgis|esri' '(?i)arcgis|esri'
P 'hkcu_esri_arcgispro_subkeys' ((@(Get-ChildItem 'HKCU:\Software\ESRI\ArcGISPro' | ForEach-Object { $_.PSChildName }) | Select-Object -First 15) -join ';')
Fam 'Siemens Solid Edge' @('Siemens', 'Siemens\Solid Edge', 'Siemens\SolidEdge') @('Siemens', 'Siemens\Solid Edge') @('Siemens', 'Siemens\Solid Edge', 'Siemens\LMS') '(?i)token|(^|[^a-z])auth([^o]|$)|credential|login|account|licen|sso|\.json$' '(?i)siemens|solid ?edge' '(?i)siemens|solid ?edge'

H 'Sign-on file mtimes (names, sizes and modification times only, newest first; never contents). Compare with the time of this boot (utc_now above): a file touched during this boot means the app contacted its identity service.'
function MT($label, $dirs) {
  $files = @()
  foreach ($d in $dirs) { if (Test-Path -LiteralPath $d) { $files += @(Get-ChildItem -LiteralPath $d -Recurse -Force -File -Depth 3 -ErrorAction SilentlyContinue) } }
  $files = @($files | Sort-Object LastWriteTimeUtc -Descending)
  P ($label + '.files_total') $files.Count
  foreach ($f in ($files | Select-Object -First 25)) {
    $kb = ''
    if ($f.Length -le 1048576 -and $f.Extension -match '(?i)^\.(json|xml|ini|txt|cfg|config|dat|db|vdf|sqlite)$') { $kb = ' contains[' + (KB $f.FullName @('access_token', 'refresh_token', 'token', 'LoggedIn', 'username', 'portal')) + ']' }
    P ($label + '.file') ('{0} bytes={1} mtime_utc={2}{3}' -f (Sh $f.FullName), $f.Length, $f.LastWriteTimeUtc.ToString('yyyy-MM-dd HH:mm'), $kb)
  }
}
$adsk = @(); $esri = @()
foreach ($u in $PR) {
  foreach ($r in @('Autodesk\Web Services', 'Autodesk\Identity Services', 'Autodesk\Autodesk Access', 'Autodesk\ODIS', 'Autodesk\AdskIdentityManager', 'Autodesk\Autodesk Identity Manager')) { $adsk += (Join-Path $u.LA $r); $adsk += (Join-Path $u.AD $r) }
  foreach ($r in @('ESRI\ArcGISPro\Settings', 'ESRI\ArcGISPro\Portals', 'ESRI\ArcGISPro\Licensing', 'ESRI\Portal')) { $esri += (Join-Path $u.LA $r); $esri += (Join-Path $u.AD $r) }
}
foreach ($r in @('Autodesk\ODIS', 'Autodesk\AdskLicensingService', 'Autodesk\CLM', 'Autodesk\Autodesk Access')) { $adsk += (Join-Path $PDATA $r) }
MT 'autodesk_signon' $adsk
MT 'esri_signon' $esri

H 'Browsers (sizes only; Login Data row count is inferred from file size)'
function Browser($name, $rel) {
  foreach ($u in $PR) {
    $ud = Join-Path $u.LA $rel
    if (-not (Test-Path -LiteralPath $ud)) { P ($name + ' ' + $u.Tag) 'no user-data dir'; continue }
    foreach ($pd in @(Get-ChildItem -LiteralPath $ud -Directory -Force | Where-Object { $_.Name -eq 'Default' -or $_.Name -like 'Profile *' })) {
      $ck = 0; foreach ($c in @('Network\Cookies', 'Cookies')) { $f = Join-Path $pd.FullName $c; if (Test-Path -LiteralPath $f) { $ck = (Get-Item -LiteralPath $f -Force).Length; break } }
      $ld = 0; $f = Join-Path $pd.FullName 'Login Data'; if (Test-Path -LiteralPath $f) { $ld = (Get-Item -LiteralPath $f -Force).Length }
      $hi = 0; $f = Join-Path $pd.FullName 'History'; if (Test-Path -LiteralPath $f) { $hi = (Get-Item -LiteralPath $f -Force).Length }
      $acct = 'unknown'; $f = Join-Path $pd.FullName 'Preferences'
      if (Test-Path -LiteralPath $f) { $acct = Yn (Select-String -LiteralPath $f -Pattern '"account_info"\s*:\s*\[\s*\{' -Quiet) }
      $bucket = if ($ld -le 0) { 'none' } elseif ($ld -le 45056) { 'empty-or-near-empty (size heuristic)' } else { 'likely-has-rows (size heuristic)' }
      P ($name + ' ' + $u.Tag + ' profile=' + ($pd.Name -replace '\s', '_')) ('cookies_bytes=' + $ck + ' login_data_bytes=' + $ld + ' [' + $bucket + '] history_bytes=' + $hi + ' signed_in_account_info_in_Preferences=' + $acct)
      Sm ('browser.' + $name + '.' + $u.Tag + '.' + ($pd.Name -replace '\s', '_') + '.account_info') $acct
    }
  }
}
Browser 'Edge' 'Microsoft\Edge\User Data'
Browser 'Chrome' 'Google\Chrome\User Data'
foreach ($u in $PR) { $fp = Join-Path $u.AD 'Mozilla\Firefox\Profiles'; if (Test-Path -LiteralPath $fp) { foreach ($d in (Get-ChildItem -LiteralPath $fp -Directory)) { $lj = Join-Path $d.FullName 'logins.json'; P ('Firefox ' + $u.Tag) ('logins.json bytes=' + $(if (Test-Path -LiteralPath $lj) { (Get-Item -LiteralPath $lj).Length } else { 0 })) } } }

H 'Name scan: files named like *token*, *auth*, *credential*, *.vdf, *ssfn*, LoginState, loginusers, oauth, *.ims (names, sizes, mtimes only)'
$roots = @(); foreach ($u in $PR) { $roots += $u.Root }; $roots += $PDATA
$skip = '(?i)^(Cache|Code Cache|GPUCache|Service Worker|IndexedDB|Crashpad|Temp|node_modules|WinSxS|Packages|Extensions|Local Storage|Session Storage|Application Cache|DawnCache|GrShaderCache|ShaderCache|common|Package Cache|Installer|DriverStore|ConnectedDevicesPlatform|INetCache|WebCache|Windows|Microsoft Edge Core)$'
$stack = New-Object System.Collections.Generic.Stack[object]
foreach ($r in $roots) { if (Test-Path -LiteralPath $r) { $stack.Push(@($r, 0)) } }
$found = New-Object System.Collections.Generic.List[object]; $seen = 0
while ($stack.Count -gt 0 -and $seen -lt 600000) {
  $cur = $stack.Pop()
  try {
    $di = New-Object IO.DirectoryInfo($cur[0])
    foreach ($f in $di.GetFiles()) { $seen++; if ($f.Name -match $RXNAME) { $found.Add($f) } }
    if ($cur[1] -lt 8) { foreach ($sd in $di.GetDirectories()) { if (($sd.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { continue }; if ($sd.Name -match $skip) { continue }; $stack.Push(@($sd.FullName, ($cur[1] + 1))) } }
  } catch { }
}
P 'files_visited' $seen
P 'named_like_token_auth_credential_vdf_ssfn_total' $found.Count
foreach ($cls in @('token', 'auth', 'credential', '\.vdf$', 'ssfn', 'loginstate|loginusers', 'oauth', '\.ims$')) { P ('class ' + $cls) (@($found | Where-Object { $_.Name -match ('(?i)' + $cls) })).Count }
foreach ($f in ($found | Sort-Object FullName | Select-Object -First 150)) { P 'file' ('{0} bytes={1} mtime={2}' -f (Sh $f.FullName), $f.Length, $f.LastWriteTimeUtc.ToString('yyyy-MM-dd')) }
if ($found.Count -gt 150) { P 'file_list_truncated_at' 150 }
Sm 'namescan.total' $found.Count

H 'SUMMARY (presence only; the operator's UI test decides PASS or FAIL)'
foreach ($k in $SUM.Keys) { P ('SUMMARY ' + $k) $SUM[$k] }
Write-Output 'KF_TC_REPORT_END'
