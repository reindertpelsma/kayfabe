# Copied from apps_b/launch.ps1 (scrub + UI Automation walk); keep in step with it.
# ---- scrub: ONE function for everything this script writes (the host scrubs again with the same patterns, propimage_apps_b.scrub_text)
$Script:ScrubRes = @(
  '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}',
  '\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\b',
  '\b[0-9A-Fa-f]{16,}\b',
  '\b[A-Za-z0-9]{4,6}(?:-[A-Za-z0-9]{4,6}){2,}\b'
)
$Script:ScrubAfter = '(?i)\b(signed in as|welcome|hello)\b.*$'
function Scrub-Text([string]$s) {
  if ([string]::IsNullOrEmpty($s)) { return '' }
  $s = $s -replace '[\r\n]+', ' '
  foreach ($re in $Script:ScrubRes) { $s = [regex]::Replace($s, $re, '<scrubbed>') }
  return [regex]::Replace($s, $Script:ScrubAfter, '$1 <scrubbed>')
}

# ---- UI text capture (UI Automation, ControlType | Name | AutomationId-short; names only, no screenshot)
$Script:HitWords = @('download', 'install', 'online', 'model', 'not available', 'unavailable', 'requires internet', 'internet connection', 'internet', 'sign in',
  'sign-in', 'signin', 'log in', 'trial', 'offline', 'update', 'connect', 'licen', 'subscription', 'get started', 'free', 'activate', 'add-on',
  'addon', 'plug-in', 'plugin', 'content', 'library', 'cloud', 'account')
$Script:UiSeen = @{}
$Script:UiLines = New-Object System.Collections.ArrayList
$Script:UiErrors = @()
$Script:UiBytes = 0
# The walk runs in a runspace of its own: one UI Automation call can block for minutes (see Dismiss-Dialogs), and a blocked walk is simply abandoned.
$Script:UiWalk = {
  param($idList, $title, $maxEl, $capS, $sink)
  Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $ids = @($idList -split ',' | Where-Object { $_ } | ForEach-Object { [int]$_ })
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  $queue = New-Object System.Collections.Queue
  $top = $walker.GetFirstChild([System.Windows.Automation.AutomationElement]::RootElement)
  while ($top -ne $null -and $sw.Elapsed.TotalSeconds -lt $capS) {
    $take = $false
    try {
      if ($ids -contains $top.Current.ProcessId) { $take = $true }
      elseif ($title -and $top.Current.Name.IndexOf($title, [StringComparison]::OrdinalIgnoreCase) -ge 0) { $take = $true }
    } catch {}
    if ($take) { $queue.Enqueue(@($top, 0)) }
    $top = $walker.GetNextSibling($top)
  }
  $n = 0
  while ($queue.Count -gt 0 -and $n -lt $maxEl -and $sw.Elapsed.TotalSeconds -lt $capS) {
    $item = $queue.Dequeue()
    $el = $item[0]
    $depth = $item[1]
    try {
      $cur = $el.Current
      $nm = [string]$cur.Name
      if ($nm.Length -gt 400) { $nm = $nm.Substring(0, 400) }
      [void]$sink.Add(($cur.ControlType.ProgrammaticName -replace '^ControlType\.', '') + [char]9 + $nm + [char]9 + [string]$cur.AutomationId)
      $n++
    } catch {}
    if ($depth -lt 14) {
      try {
        $ch = $walker.GetFirstChild($el)
        while ($ch -ne $null -and $sw.Elapsed.TotalSeconds -lt $capS) {
          $queue.Enqueue(@($ch, ($depth + 1)))
          $ch = $walker.GetNextSibling($ch)
        }
      } catch {}
    }
  }
}
# The process ids of the application and of its child processes (up to three levels).
function Get-AppPids([string]$proc) {
  $found = @(Proc-Pids $proc)
  try {
    $all = @(Get-CimInstance Win32_Process -ErrorAction Stop | Select-Object ProcessId, ParentProcessId)
    for ($i = 0; $i -lt 3; $i++) {
      $kids = @($all | Where-Object { ($found -contains $_.ParentProcessId) -and ($found -notcontains $_.ProcessId) } | ForEach-Object { $_.ProcessId })
      if (-not $kids.Count) { break }
      $found += $kids
    }
  } catch {}
  return @($found | Select-Object -Unique)
}
# One walk, hard-capped at $capS seconds: the lines gathered so far are returned even when the walk was abandoned.
function Capture-UiText([string]$proc, [string]$title, [int]$maxEl = 3000, [int]$capS = 60) {
  $sink = [System.Collections.ArrayList]::Synchronized((New-Object System.Collections.ArrayList))
  try {
    $ids = (Get-AppPids $proc) -join ','
    $ps = [powershell]::Create()
    [void]$ps.AddScript($Script:UiWalk.ToString())
    [void]$ps.AddArgument($ids)
    [void]$ps.AddArgument($title)
    [void]$ps.AddArgument($maxEl)
    [void]$ps.AddArgument($capS)
    [void]$ps.AddArgument($sink)
    $ar = $ps.BeginInvoke()
    $t0 = Get-Date
    while (-not $ar.IsCompleted -and ((Get-Date) - $t0).TotalSeconds -lt ($capS + 5)) { Start-Sleep -Milliseconds 500 }
    if (-not $ar.IsCompleted) {
      $Script:UiErrors += 'walk cap reached'
      try { [void]$ps.BeginStop($null, $null) } catch {}
    }
  } catch { $Script:UiErrors += ('walk: ' + (Scrub-Text ($_.Exception.Message))) }
  return @($sink.ToArray())
}
