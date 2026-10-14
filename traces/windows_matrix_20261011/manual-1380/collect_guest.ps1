# Read-only post-manual observation. No sign-in, input, registry writes or apps started.
$ErrorActionPreference = 'Continue'
$since = ([DateTimeOffset]'2026-10-11T20:17:45Z').LocalDateTime
$o = @{ utc = (Get-Date).ToUniversalTime().ToString('o'); run = 1380 }
$o.boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o')
$o.display = @(Get-PnpDevice -Class Display -ErrorAction SilentlyContinue | ForEach-Object {
    @{ name = $_.FriendlyName; status = [string]$_.Status; problem = [string]$_.Problem }
})
$events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $since } -ErrorAction SilentlyContinue |
    Where-Object { $_.ProviderName -match 'nvlddmkm|Display|DxgKrnl' -or $_.Id -eq 141 } |
    Select-Object -First 48)
$o.system_events = @(foreach ($event in $events) {
    $stamp = if ($event.TimeCreated) { $event.TimeCreated.ToUniversalTime().ToString('o') } else { 'UNMEASURED' }
    $message = [string]$event.Message
    @{ utc = $stamp; provider = $event.ProviderName; id = $event.Id;
       message = $message.Substring(0, [Math]::Min(2000, $message.Length)) }
})
$o.system_events_selected = $events.Count
$o.app_event_counts = @{}
foreach ($id in @(1000, 1001, 1002)) {
    $o.app_event_counts["$id"] = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $since; Id = $id } -ErrorAction SilentlyContinue).Count
}
# Never print process command lines: game launchers can put account tokens in arguments.
$o.game_processes = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'java|minecraft' } |
    ForEach-Object { @{ name = $_.Name; pid = $_.Id; responding = $_.Responding; cpu_s = $_.CPU } })
Write-Output ('MANUAL1380 ' + (ConvertTo-Json $o -Depth 6 -Compress))
