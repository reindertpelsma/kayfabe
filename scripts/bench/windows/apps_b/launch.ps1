# Launch-and-main-window check for the applications that have no scripting route usable offline: start the app, handle first-run
# dialogs, wait for the main window through UI Automation, hold it for a while, record the process state and the app's own GPU
# sniffer output (when it ships one), then close it cleanly. $APPNAME is set by the caller (header line).
# 2026-11-20 (new full B image, discovery lane): rows with `exe = $null` have an install directory and an exe name that carry versions we do not
# know, so the exe is DISCOVERED from the file-name patterns of `find` (first match wins, never a versioned path). Every row also writes
# inventory.txt (NAMES of the folders and exes under C:\Program Files\Adobe*) and, after the main window and a 30 s hold and again after 60 s more,
# the UI TEXT of the application's windows (uitext.txt, uitext_hits.txt: names of controls read through UI Automation, scrubbed; never a screenshot).
# The capture is best effort: it never fails or hangs the row and never changes the verdict (a failure is recorded as uitext_error).
. 'C:\kfhw\lib.ps1'
$T = @{
  premiere     = @{ exe = 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\Adobe Premiere Pro.exe'; proc = 'Adobe Premiere Pro'; title = 'Premiere'; sniffer = 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\GPUSniffer.exe' }
  mediaencoder = @{ exe = 'C:\Program Files\Adobe\Adobe Media Encoder 2027\Adobe Media Encoder.exe'; proc = 'Adobe Media Encoder'; title = 'Media Encoder'; sniffer = 'C:\Program Files\Adobe\Adobe Media Encoder 2027\GPUSniffer.exe' }
  lightroomlaunch = @{ exe = 'C:\Program Files\Adobe\Adobe Lightroom Classic\Lightroom.exe'; proc = 'Lightroom'; title = 'Lightroom'; sniffer = $null }
  dimension    = @{ exe = 'C:\Program Files\Adobe\Adobe Dimension\Adobe Dimension.exe'; proc = 'Adobe Dimension'; title = 'Dimension'; sniffer = 'C:\Program Files\Adobe\Adobe Dimension\gpu-sniffer-core.exe' }
  uxp          = @{ exe = 'C:\Program Files\Adobe\Adobe UXP Developer Tools\Adobe UXP Developer Tools.exe'; proc = 'Adobe UXP Developer Tools'; title = 'UXP'; sniffer = $null }
  bridge       = @{ exe = 'C:\Program Files\Adobe\Adobe Bridge 2027\Adobe Bridge.exe'; proc = 'Adobe Bridge'; title = 'Bridge'; sniffer = $null }
  characteranimator = @{ exe = 'C:\Program Files\Adobe\Adobe Character Animator 2026\Support Files\Character Animator.exe'; proc = 'Character Animator'; title = 'Character Animator'; sniffer = $null }
  audition     = @{ exe = 'C:\Program Files\Adobe\Adobe Audition 2027\Adobe Audition.exe'; proc = 'Adobe Audition'; title = 'Audition'; sniffer = $null }
  dreamweaver  = @{ exe = $null; find = @('Dreamweaver*.exe'); proc = $null; title = 'Dreamweaver'; sniffer = $null }
  incopy       = @{ exe = $null; find = @('InCopy*.exe', '*InCopy*.exe'); proc = $null; title = 'InCopy'; sniffer = $null }
  substance_designer = @{ exe = $null; find = @('*Substance*3D*Designer*.exe', '*Substance*Designer*.exe'); proc = $null; title = 'Designer'; sniffer = $null }
  substance_modeler = @{ exe = $null; find = @('*Substance*3D*Modeler*.exe', '*Substance*Modeler*.exe'); proc = $null; title = 'Modeler'; sniffer = $null }
  substance_painter = @{ exe = $null; find = @('*Substance*3D*Painter*.exe', '*Substance*Painter*.exe'); proc = $null; title = 'Painter'; sniffer = $null }
  substance_sampler = @{ exe = $null; find = @('*Substance*3D*Sampler*.exe', '*Substance*Sampler*.exe'); proc = $null; title = 'Sampler'; sniffer = $null }
  substance_stager = @{ exe = $null; find = @('*Substance*3D*Stager*.exe', '*Substance*Stager*.exe'); proc = $null; title = 'Stager'; sniffer = $null }
  mediaencoderlaunch = @{ exe = $null; find = @('Adobe Media Encoder*.exe'); proc = $null; title = 'Media Encoder'; sniffer = $null }
  animatelaunch = @{ exe = $null; find = @('Animate.exe', 'Adobe Animate*.exe'); proc = $null; title = 'Animate'; sniffer = $null }
}
$R = Init-Run $APPNAME
$c = $T[$APPNAME]
$deadline = (Get-Date).AddSeconds(700)
function Secs-Left { return [int][Math]::Max(0, ($deadline - (Get-Date)).TotalSeconds) }

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

# ---- inventory: names only (folders under C:\Program Files\Adobe*, exes directly in them or one level down), bounded
function Write-Inventory {
  try {
    $lines = @()
    foreach ($root in @(Get-ChildItem -LiteralPath 'C:\Program Files' -Directory -Filter 'Adobe*' -ErrorAction SilentlyContinue)) {
      $lines += ('root: ' + $root.Name)
      foreach ($e in @(Get-ChildItem -LiteralPath $root.FullName -File -Filter '*.exe' -ErrorAction SilentlyContinue)) { $lines += ('  exe: ' + $e.Name) }
      foreach ($d in @(Get-ChildItem -LiteralPath $root.FullName -Directory -ErrorAction SilentlyContinue)) {
        $lines += ('  folder: ' + $d.Name)
        foreach ($e in @(Get-ChildItem -LiteralPath $d.FullName -File -Filter '*.exe' -ErrorAction SilentlyContinue)) { $lines += ('    exe: ' + $e.Name) }
        foreach ($s in @(Get-ChildItem -LiteralPath $d.FullName -Directory -ErrorAction SilentlyContinue)) {
          foreach ($e in @(Get-ChildItem -LiteralPath $s.FullName -File -Filter '*.exe' -ErrorAction SilentlyContinue)) { $lines += ('    exe: ' + $s.Name + '\' + $e.Name) }
        }
        if ($lines.Count -gt 1500) { break }
      }
    }
    $out = @($lines | Select-Object -First 1500 | ForEach-Object { Scrub-Text $_ })
    $enc = New-Object System.Text.UTF8Encoding($false)
    $text = ($out -join "`r`n")
    if ($text.Length -gt 60000) { $text = $text.Substring(0, 60000) }
    [System.IO.File]::WriteAllText("$D\inventory.txt", $text + "`r`n", $enc)
  } catch { $R.inventory_error = Scrub-Text ($_.Exception.Message) }
}

# ---- discovery: the first exe whose NAME matches a pattern (patterns in order), under C:\Program Files\Adobe* and C:\Program Files\Adobe\*, depth 3
$Script:ExeSkip = '(?i)unins|uninstall|crash|helper|updater|install|setup|sniffer|broker|service|cli\.exe$|node\.exe$'
function Find-AppExe([string[]]$patterns) {
  $roots = @()
  foreach ($a in @(Get-ChildItem -LiteralPath 'C:\Program Files' -Directory -Filter 'Adobe*' -ErrorAction SilentlyContinue)) { $roots += $a.FullName }
  foreach ($b in @(Get-ChildItem -LiteralPath 'C:\Program Files\Adobe' -Directory -ErrorAction SilentlyContinue)) { $roots += $b.FullName }
  $seen = @{}
  $all = @()
  foreach ($root in $roots) {
    foreach ($f in @(Get-ChildItem -LiteralPath $root -Recurse -Depth 3 -File -Filter '*.exe' -ErrorAction SilentlyContinue)) {
      if ($seen.ContainsKey($f.FullName) -or $f.Name -match $Script:ExeSkip) { continue }
      $seen[$f.FullName] = $true
      $all += $f
    }
    if ($all.Count -gt 5000) { break }
  }
  foreach ($p in $patterns) {
    $hit = @($all | Where-Object { $_.Name -like $p } | Sort-Object { ($_.FullName -split '\\').Count }, FullName | Select-Object -First 1)
    if ($hit.Count) { return $hit[0].FullName }
  }
  return $null
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
function Add-UiText([string]$proc, [string]$title) {
  try {
    if ((Secs-Left) -lt 30) { $Script:UiErrors += 'no time left for a walk'; return }
    $cap = [Math]::Min(60, (Secs-Left))
    foreach ($raw in @(Capture-UiText $proc $title 3000 $cap)) {
      $p = ([string]$raw).Split([char]9)
      if ($p.Count -lt 2) { continue }
      $nm = ($p[1] -replace '\s+', ' ').Trim()
      if (-not $nm) { continue }
      $ct = $p[0]
      $aid = ''
      if ($p.Count -gt 2) { $aid = Scrub-Text (($p[2] -replace '\s+', ' ').Trim()) }
      if ($aid.Length -gt 40) { $aid = $aid.Substring(0, 40) }
      $nm = Scrub-Text $nm
      if ($nm.Length -gt 300) { $nm = $nm.Substring(0, 300) }
      $key = $ct + '|' + $nm
      if ($Script:UiSeen.ContainsKey($key)) { continue }
      $line = Scrub-Text ('{0} | {1} | {2}' -f $ct, $nm, $aid)
      if ($Script:UiBytes + $line.Length + 2 -gt 200000) { $Script:UiErrors += 'uitext bound reached'; break }
      $Script:UiSeen[$key] = $true
      [void]$Script:UiLines.Add($line)
      $Script:UiBytes += $line.Length + 2
    }
  } catch { $Script:UiErrors += ('capture: ' + (Scrub-Text ($_.Exception.Message))) }
}
function Write-UiFiles {
  try {
    $enc = New-Object System.Text.UTF8Encoding($false)
    $lines = [string[]]@($Script:UiLines)
    [System.IO.File]::WriteAllLines("$D\uitext.txt", $lines, $enc)
    $rx = '(?i)(' + ((@($Script:HitWords) | ForEach-Object { [regex]::Escape($_) }) -join '|') + ')'
    $hits = [string[]]@($lines | Where-Object { $_ -match $rx })
    [System.IO.File]::WriteAllLines("$D\uitext_hits.txt", $hits, $enc)
    $R.ui_lines = $lines.Count
    $R.ui_hits = $hits.Count
  } catch { $Script:UiErrors += ('write: ' + (Scrub-Text ($_.Exception.Message))) }
}
function Set-UiError {
  if ($Script:UiErrors.Count) {
    $msg = Scrub-Text ((@($Script:UiErrors) | Select-Object -Unique) -join '; ')
    if ($msg.Length -gt 200) { $msg = $msg.Substring(0, 200) }
    $R.uitext_error = $msg
  }
}

try {
  Write-Inventory
  if (-not $c.exe) {
    $found = Find-AppExe $c.find
    if (-not $found) { throw 'not installed' }
    $c.exe = $found
    $c.proc = [System.IO.Path]::GetFileNameWithoutExtension($found)
  } elseif (-not (Test-Path -LiteralPath $c.exe)) {
    throw 'not installed'
  }
  $R.exe_dir = Split-Path (Split-Path $c.exe -Parent) -Leaf
  $R.exe_name = Split-Path $c.exe -Leaf
  $waitS = 600
  if ($c.find) { $waitS = 360 }
  if ($c.sniffer -and (Test-Path $c.sniffer)) {
    ((cmd.exe /c "`"$($c.sniffer)`" 2>&1") | Out-String) | Set-Content -Encoding UTF8 "$D\sniffer.txt"
  }
  Step 'launch'
  Start-Process $c.exe
  $ok = Wait-For { Has-Window $c.proc } $c.proc $waitS
  $R.main_window = $ok
  Step "window=$ok"
  [void](Wait-For { $false } $c.proc 30)
  $R.titles = @(Win-Titles $c.proc | Where-Object { $_ } | ForEach-Object { Scrub-Text $_ })
  $pr = Get-Process $c.proc -ErrorAction SilentlyContinue | Select-Object -First 1
  $R.alive = [bool]$pr
  $R.responding = if ($pr) { [bool]$pr.Responding } else { $false }
  $R.private_mb = if ($pr) { [int]($pr.PrivateMemorySize64 / 1MB) } else { 0 }
  $R.outputs = @(); if (Test-Path "$D\sniffer.txt") { $R.outputs += (File-Info "$D\sniffer.txt") }
  Add-UiText $c.proc $c.title
  $hold = [Math]::Min(60, (Secs-Left))
  [void](Wait-For { $false } $c.proc $hold)
  Add-UiText $c.proc $c.title
  Write-UiFiles
  Set-UiError
  $R.quit = Quit-App $c.proc
  if (-not $ok) { throw 'main window did not appear' }
} catch {
  $R.error = $_.Exception.Message
  Set-UiError
  if (-not $R.quit) { $R.quit = if ($c.proc) { Quit-App $c.proc } else { 'not started' } }
}
# the dialogs the row dismissed are listed in result.json (window title: button): scrub them like everything else
$Script:Dismissed = @($Script:Dismissed | ForEach-Object { Scrub-Text $_ })
Finish $R
