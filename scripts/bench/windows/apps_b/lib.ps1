# Guest-side library for the application checks (PowerShell 5.1, run in the interactive desktop session at the user's normal
# (non-elevated) integrity level: COM activation of the applications fails from an elevated task).
# Dot-sourced by every check; checks set $D (run directory), $DATA (read-only data volume root) and fill $R, then call Finish.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
function Step($m) { ("{0} {1}" -f (Get-Date -Format HH:mm:ss), $m) | Add-Content "$D\steps.log" }
function Epoch { ([DateTimeOffset](Get-Date).ToUniversalTime()).ToUnixTimeMilliseconds() / 1000.0 }
function Data-Root {
  $v = Get-Volume | Where-Object { $_.FileSystemLabel -eq 'KFDATA' } | Select-Object -First 1
  if (-not $v) { throw 'data volume KFDATA not attached' }
  return ($v.DriveLetter + ':\')
}
function Find-Asset($name) { $r = Data-Root; $f = Get-ChildItem $r -Recurse -Filter $name | Select-Object -First 1; if (-not $f) { throw "asset $name not found on the data volume" }; return $f }
function Proc-Pids($names) { @(Get-Process $names -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }
# Names of the buttons that close a first-run, tip, recovery or graphics-notice dialog without choosing anything harmful.
$Script:DialogButtons = @('OK', 'Ok', 'Continue', 'Not Now', 'Skip', 'Got it', 'Got It', 'Maybe Later', 'No Thanks', 'No thanks',
  'Skip All', 'Done', 'Later', 'Remind Me Later', 'Launch Illustrator', 'Launch InDesign', "Don't Save", 'Do not show again', 'Start',
  'Accept', 'Agree', 'I Agree', 'Continue Anyway', 'Not now', 'Dismiss')
# Buttons that are never pressed even when listed above: they could start a sign-in or an online flow.
$Script:NeverButtons = @('Sign In', 'Sign in', 'Sign Up', 'Create an account', 'Get Started', 'Buy', 'Subscribe', 'Try')
function Dismiss-Dialogs($procNames, [string[]]$only = $null) {
  # Two levels only (a process window, its child windows, their direct children): a full subtree walk of a large application window can
  # block in UI Automation for minutes (MEASURED with Premiere Pro under kf3), and every dialog seen so far is a window one level down.
  $pids = Proc-Pids $procNames
  $roots = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
  $n = 0
  $list = if ($only) { $only } else { $Script:DialogButtons }
  foreach ($r in $roots) {
    try { if ($pids -notcontains $r.Current.ProcessId) { continue } } catch { continue }
    $title = $r.Current.Name
    $level = @($r)
    try { $level += @($r.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' }) } catch {}
    foreach ($w in $level) {
      $wt = $w.Current.Name
      try { $kids = $w.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition) } catch { continue }
      foreach ($el in $kids) {
        if ($el.Current.ControlType.ProgrammaticName -ne 'ControlType.Button') { continue }
        $nm = $el.Current.Name
        if ($list -contains $nm -and $Script:NeverButtons -notcontains $nm -and $el.Current.IsEnabled) {
          try { $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); $n++; $Script:Dismissed += ("{0}: {1}" -f $wt, $nm) } catch {}
        }
      }
    }
  }
  return $n
}
$Script:Dismissed = @()
# Wait until $cond (scriptblock) is true, dismissing dialogs of the named processes meanwhile. Returns $true or $false at the timeout.
function Wait-For($cond, $procNames, [int]$timeoutS = 120, [int]$pollS = 2) {
  $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt $timeoutS) {
    if (& $cond) { return $true }
    [void](Dismiss-Dialogs $procNames)
    Start-Sleep $pollS
  }
  return [bool](& $cond)
}
function Win-Titles($procNames) {
  $pids = Proc-Pids $procNames
  $roots = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
  @(foreach ($r in $roots) { try { if ($pids -contains $r.Current.ProcessId) { $r.Current.Name } } catch {} })
}
# Keyboard fallback for dialogs whose controls UI Automation does not expose (After Effects): the key goes to the foreground window.
function Press-Key($keys) { try { [System.Windows.Forms.SendKeys]::SendWait($keys) } catch {} }
# After Effects refuses scripts that write files until the per-user preference is on (MEASURED 2026-11-16: the script aborts at its first file
# write, the application stays open with an empty project and a UI_MessageBox, nothing is saved). The preference lives in the user's own
# Prefs.txt and is changed here while the application is not running; nothing outside the user profile is touched.
function Enable-AeFileScripts {
  if (Get-Process AfterFX -ErrorAction SilentlyContinue) { throw 'After Effects is running: preference not changed' }
  $f = Get-ChildItem "$env:APPDATA\Adobe\After Effects" -Recurse -Filter 'Adobe After Effects * Prefs.txt' -ErrorAction SilentlyContinue | Where-Object { $_.Name -notmatch 'Prefs-' } | Select-Object -First 1
  if (-not $f) {
    # a fresh clone has no preferences file until the application has run once: start it, wait for the file, close it
    $base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
    Start-Process "$base\AfterFX.exe" | Out-Null
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 600 -and -not $f) {
      Start-Sleep 5; [void](Dismiss-Dialogs 'AfterFX'); Press-Key '{ESC}'
      $f = Get-ChildItem "$env:APPDATA\Adobe\After Effects" -Recurse -Filter 'Adobe After Effects * Prefs.txt' -ErrorAction SilentlyContinue | Where-Object { $_.Name -notmatch 'Prefs-' } | Select-Object -First 1
    }
    [void](Quit-App 'AfterFX')
    if (-not $f) { return 'no Prefs.txt after a first start' }
  }
  $t = Get-Content $f.FullName -Raw
  # also: no "Tip of the Day" window at the start (a modal window the script's app.quit() cannot get past; MEASURED 2026-11-16)
  $n = $t -replace '"Show Tip of the Day" = 01', '"Show Tip of the Day" = 00'
  if ($n -notmatch '"Pref_SCRIPTING_FILE_NETWORK_SECURITY" = "1"') { $n = $n -replace '"Pref_SCRIPTING_FILE_NETWORK_SECURITY" = "0"', '"Pref_SCRIPTING_FILE_NETWORK_SECURITY" = "1"' }
  if ($n -eq $t) { return $(if ($t -match '"Pref_SCRIPTING_FILE_NETWORK_SECURITY" = "1"') { 'already on' } else { 'key not found' }) }
  Set-Content -Path $f.FullName -Value $n -NoNewline
  return 'switched on'
}
# Run an ExtendScript file in After Effects, Premiere Pro or Media Encoder. `AfterFX.exe -r script` is unreliable (MEASURED 2026-11-16: ignored in
# 2 of 4 cold starts, and also ignored when handed to an idle instance on a fresh clone). So the script is handed over by a STARTUP stub: each
# host runs every *.jsx of its Scripts\Startup folder at launch; the stub (installed ELEVATED by the harness before the row, see
# propimage_apps_b.startup_stub_setup) reads C:\kfhw\<app>_job.txt, deletes it and runs the script named there after a short delay. Each attempt
# starts a fresh application; the script must write `$marker` as its last act. A missing marker kills the application and retries.
function Run-StartupScript($app, $exe, $proc, $jsx, $marker, [int]$timeoutS = 900, [int]$attempts = 3) {
  $R.startup_attempts = @()
  $job = "C:\kfhw\${app}_job.txt"
  for ($att = 1; $att -le $attempts; $att++) {
    Get-Process $proc -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item $marker -ErrorAction SilentlyContinue
    Set-Content $job ($jsx -replace '\\', '/') -NoNewline
    $t0 = Get-Date
    Start-Process $exe | Out-Null
    $n = 0; $limit = [int]($timeoutS / $attempts) + 60
    while (-not (Test-Path $marker) -and ((Get-Date) - $t0).TotalSeconds -lt $limit) {
      [void](Dismiss-Dialogs $proc); Start-Sleep 3; $n++
      if ($n % 15 -eq 0) { Press-Key '{ESC}' }
    }
    $ok = Test-Path $marker
    $R.startup_attempts += ("{0}:{1}:{2}s" -f $att, $(if ($ok) { 'marker' } else { 'none' }), [int]((Get-Date) - $t0).TotalSeconds)
    if ($ok) { break }
  }
  if (Test-Path "C:\kfhw\${app}_stub.log") { Copy-Item "C:\kfhw\${app}_stub.log" "$D\${app}_stub.log" -Force }
  if (-not (Test-Path $marker)) { throw ("$proc did not run the script in $attempts attempts (no marker; stub log " + $(if (Test-Path "$D\${app}_stub.log") { 'present' } else { 'absent' }) + ')') }
  $t1 = Get-Date; $k = 0
  while ((Get-Process $proc -ErrorAction SilentlyContinue) -and ((Get-Date) - $t1).TotalSeconds -lt 120) { Start-Sleep 3; $k++; if ($k % 5 -eq 0) { Press-Key '{ESC}' } }
}
function Run-AeScript($jsx, $marker, [int]$timeoutS = 900, [int]$attempts = 3) {
  $R.ae_file_scripts = Enable-AeFileScripts
  Run-StartupScript 'ae' 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files\AfterFX.exe' 'AfterFX' $jsx $marker $timeoutS $attempts
}
# Wait for a process started with Start-Process -PassThru and redirected output: Process.WaitForExit()/HasExited can stay false for ever while a
# grandchild keeps the redirected handles open (MEASURED with aerender), so poll the process table by id instead. Returns $true when it ended.
function Wait-Proc($p, [int]$timeoutMs) {
  $t0 = Get-Date
  while (((Get-Date) - $t0).TotalMilliseconds -lt $timeoutMs) {
    if (-not (Get-Process -Id $p.Id -ErrorAction SilentlyContinue)) { return $true }
    Start-Sleep 2
  }
  return $false
}
# aerender sometimes starts and never launches its renderer (MEASURED: 0 CPU seconds in 7 minutes, no AfterFX child, no frame). A start that has
# produced no frame (and no new log line) after $stallS seconds is killed and started again, up to $attempts times; the attempts are recorded.
function Run-Aerender($argList, $log, $framesDir, [int]$budgetMs = 900000, [int]$stallS = 150, [int]$attempts = 3) {
  $R.aerender_attempts = @()
  for ($a = 1; $a -le $attempts; $a++) {
    Get-Process aerender -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    $ar = Start-Process 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files\aerender.exe' -ArgumentList $argList -RedirectStandardOutput $log -RedirectStandardError "$log.err" -PassThru -WindowStyle Hidden
    $t0 = Get-Date; $stalled = $false
    while (Get-Process -Id $ar.Id -ErrorAction SilentlyContinue) {
      $el = ((Get-Date) - $t0).TotalSeconds
      if ($el -gt ($budgetMs / 1000)) { break }
      $fr = @(Get-ChildItem $framesDir -ErrorAction SilentlyContinue).Count
      if ($el -gt $stallS -and $fr -eq 0 -and -not (Get-Process AfterFX -ErrorAction SilentlyContinue)) { $stalled = $true; break }
      Start-Sleep 3
    }
    $R.aerender_attempts += ("{0}:{1}:{2}s" -f $a, $(if ($stalled) { 'stalled' } else { 'ran' }), [int]((Get-Date) - $t0).TotalSeconds)
    if (-not $stalled) { return $ar }
    Stop-Process -Id $ar.Id -Force -ErrorAction SilentlyContinue
    Get-Process AfterFX -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep 5
  }
  return $ar
}
function Has-Window($procNames) { (@(Win-Titles $procNames | Where-Object { $_ })).Count -gt 0 }
function Quit-App($procNames, $comApp = $null, [int]$budget = 150) {
  # clean exit: ask politely first, then close the windows, only then end the process (and say which one happened)
  if ($comApp) { try { $comApp.Quit() } catch { try { $comApp.Exit() } catch {} } }
  $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt $budget) {
    if (-not (Proc-Pids $procNames)) { return 'clean' }
    foreach ($p in Get-Process $procNames -ErrorAction SilentlyContinue) { [void]$p.CloseMainWindow() }
    [void](Dismiss-Dialogs $procNames @("Don't Save", 'No', 'Discard', 'OK', 'Skip This Time', 'Skip', 'Exit', 'Quit'))
    Start-Sleep 2
  }
  foreach ($p in Get-Process $procNames -ErrorAction SilentlyContinue) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  return 'forced'
}
function Finish($R) {
  $R.ended = Epoch
  # crash evidence of this run: Application log errors/WER entries since the row started, and the minidumps WER kept (names and sizes only)
  try {
    $since = [DateTimeOffset]::FromUnixTimeSeconds([long]$R.started).UtcDateTime
    $ev = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $since.ToLocalTime() } -ErrorAction SilentlyContinue | Where-Object { $_.Id -in 1000, 1001, 1002, 1005, 1026, 4097 -or $_.ProviderName -match 'Windows Error Reporting|Application Error|Application Hang' } | Select-Object -First 12)
    $tx = $ev | ForEach-Object { ("{0} id={1} {2}: {3}" -f $_.TimeCreated.ToString('HH:mm:ss'), $_.Id, $_.ProviderName, (($_.Message -replace '\s+', ' ') -replace 'Attached files:.*$', '')) }
    $dm = @(Get-ChildItem 'C:\kfapps\dumps' -Filter *.dmp -ErrorAction SilentlyContinue | ForEach-Object { 'dump {0} {1} bytes {2}' -f $_.Name, $_.Length, $_.LastWriteTime.ToString('HH:mm:ss') })
    if ($tx -or $dm) { Set-Content "$D\app_events.txt" (@($tx) + @($dm)) -Encoding UTF8; $R.crash_events = @($ev).Count; $R.crash_dumps = @($dm).Count }
  } catch { $R.events_error = $_.Exception.Message }
  $R.dismissed = @($Script:Dismissed)
  ($R | ConvertTo-Json -Depth 6) | Set-Content -Encoding UTF8 "$D\result.json"
}
function Init-Run($app) {
  New-Item -Force -ItemType Directory $D | Out-Null
  return [ordered]@{ app = $app; started = (Epoch); error = $null; outputs = @() }
}
function File-Info($path) { $f = Get-Item $path; return [ordered]@{ file = $f.Name; bytes = $f.Length; modified = ([DateTimeOffset]$f.LastWriteTimeUtc).ToUnixTimeMilliseconds() / 1000.0 } }
