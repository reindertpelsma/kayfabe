# System facts of the offline boot test (run over ssh as the guest user; read-only). Prints key=value lines.
$ErrorActionPreference = 'Continue'
'utc_now=' + (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
'tz=' + (Get-TimeZone).Id
'boot_time_utc=' + (Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
'os=' + (Get-CimInstance Win32_OperatingSystem).Caption + ' ' + (Get-CimInstance Win32_OperatingSystem).Version
'user=' + $env:USERNAME
'session=' + ((query user 2>&1) -join ' / ')
'nvsmi_L=' + ((& nvidia-smi -L 2>&1) -join ' ; ')
'nvsmi=' + ((& nvidia-smi --query-gpu=name,driver_version,memory.total,pstate,utilization.gpu --format=csv,noheader 2>&1) -join ' ; ')
'display=' + ((Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name + ' ' + $_.CurrentHorizontalResolution + 'x' + $_.CurrentVerticalResolution + ' status=' + $_.Status }) -join ' ; ')
'gpu_pnp=' + ((Get-PnpDevice -Class Display -ErrorAction SilentlyContinue | ForEach-Object { $_.FriendlyName + ' ' + $_.Status }) -join ' ; ')
'routes=' + ((Get-NetRoute -AddressFamily IPv4 -ErrorAction SilentlyContinue | Where-Object { $_.DestinationPrefix -eq '0.0.0.0/0' } | ForEach-Object { 'default via ' + $_.NextHop }) -join ' ; ')
'adapters=' + ((Get-NetAdapter -ErrorAction SilentlyContinue | ForEach-Object { $_.Name + ' ' + $_.Status }) -join ' ; ')
'fw_profiles=' + ((Get-NetFirewallProfile | ForEach-Object { $_.Name + ' enabled=' + $_.Enabled + ' out=' + $_.DefaultOutboundAction }) -join ' ; ')
'fw_block_out_rules=' + ((Get-NetFirewallRule -Direction Outbound -Action Block -Enabled True -ErrorAction SilentlyContinue | Select-Object -First 8 | ForEach-Object { $_.DisplayName }) -join ' ; ')
foreach ($t in @(@('1.1.1.1', 443), @('8.8.8.8', 53), @('example.com', 443))) {
  $c = New-Object System.Net.Sockets.TcpClient
  $r = $c.BeginConnect($t[0], $t[1], $null, $null)
  $ok = $r.AsyncWaitHandle.WaitOne(6000, $false) -and $c.Connected
  ('tcp_{0}_{1}={2}' -f $t[0], $t[1], $(if ($ok) { 'CONNECTED' } else { 'blocked_or_timeout' }))
  $c.Close()
}
'dns_example.com=' + $(try { (Resolve-DnsName example.com -DnsOnly -QuickTimeout -ErrorAction Stop | Select-Object -First 1 | ForEach-Object { $_.IPAddress }) } catch { 'FAILED: ' + $_.Exception.Message.Substring(0, [Math]::Min(60, $_.Exception.Message.Length)) })
'https_example.com=' + $(try { (Invoke-WebRequest -Uri 'https://example.com' -UseBasicParsing -TimeoutSec 8).StatusCode } catch { 'FAILED' })
'ram_mb=' + [int]((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1MB)
'disk_free_gb=' + [int]((Get-PSDrive C).Free / 1GB)
'kfdata=' + ((Get-Volume | Where-Object { $_.FileSystemLabel -eq 'KFDATA' } | ForEach-Object { $_.DriveLetter + ':' }) -join ',')
