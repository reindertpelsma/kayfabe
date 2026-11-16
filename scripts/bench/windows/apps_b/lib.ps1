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
  if ($t -match '"Pref_SCRIPTING_FILE_NETWORK_SECURITY" = "1"') { return 'already on' }
  $n = $t -replace '"Pref_SCRIPTING_FILE_NETWORK_SECURITY" = "0"', '"Pref_SCRIPTING_FILE_NETWORK_SECURITY" = "1"'
  if ($n -eq $t) { return 'key not found' }
  Set-Content -Path $f.FullName -Value $n -NoNewline
  return 'switched on'
}
# Run an ExtendScript file in After Effects. `AfterFX.exe -r script` is unreliable (MEASURED 2026-11-16: ignored in 2 of 4 cold starts, and
# also ignored when handed to an idle instance on a fresh clone). So the script is handed over by a STARTUP stub instead: AE runs every
# *.jsx of its Scripts\Startup folder at launch, the stub reads a job file naming the real script and runs it after a short delay
# (app.scheduleTask). The job file is deleted by the stub, so a restart does not rerun it. Each attempt starts a fresh AE; the script must
# write `$marker` as its last act (and may quit AE itself). A missing marker kills AE and retries (3 attempts); the attempts are recorded.
function Install-AeStartupStub {
  $dirs = @('C:\Program Files\Adobe\Adobe After Effects 2027\Support Files\Scripts\Startup', "$env:APPDATA\Adobe\After Effects\27.0\Scripts\Startup")
  $stub = @'
try {
  var j = new File("C:/kfhw/ae_job.txt");
  if (j.exists) {
    j.open("r"); var p = j.read(); j.close(); j.remove();
    var l = new File("C:/kfhw/ae_stub.log"); l.open("a"); l.writeln(new Date().getTime() + " job " + p); l.close();
    app.scheduleTask("try { $.evalFile(new File('" + p + "')); } catch (e) { var f = new File('C:/kfhw/ae_stub.log'); f.open('a'); f.writeln('ERR ' + e); f.close(); }", 6000, false);
  }
} catch (e) { }
'@
  $done = @()
  foreach ($d in $dirs) {
    try { New-Item -Force -ItemType Directory $d -ErrorAction Stop | Out-Null; Set-Content -Path (Join-Path $d 'kf_job.jsx') -Value $stub -ErrorAction Stop; $done += $d } catch { }
  }
  return ($done -join ';')
}
function Run-AeScript($jsx, $marker, [int]$timeoutS = 900, [int]$attempts = 3) {
  $base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
  $R.ae_file_scripts = Enable-AeFileScripts
  $R.ae_stub_dirs = Install-AeStartupStub
  if (-not $R.ae_stub_dirs) { throw 'no Startup folder writable' }
  $R.ae_attempts = @()
  for ($att = 1; $att -le $attempts; $att++) {
    Get-Process AfterFX -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item $marker -ErrorAction SilentlyContinue
    Set-Content 'C:\kfhw\ae_job.txt' ($jsx -replace '\\', '/') -NoNewline
    $t0 = Get-Date
    Start-Process "$base\AfterFX.exe" | Out-Null
    $n = 0; $limit = [int]($timeoutS / $attempts) + 60
    while (-not (Test-Path $marker) -and ((Get-Date) - $t0).TotalSeconds -lt $limit) {
      [void](Dismiss-Dialogs 'AfterFX'); Start-Sleep 3; $n++
      if ($n % 15 -eq 0) { Press-Key '{ESC}' }
    }
    $ok = Test-Path $marker
    $R.ae_attempts += ("{0}:{1}:{2}s" -f $att, $(if ($ok) { 'marker' } else { 'none' }), [int]((Get-Date) - $t0).TotalSeconds)
    if ($ok) { break }
  }
  if (Test-Path 'C:\kfhw\ae_stub.log') { Copy-Item 'C:\kfhw\ae_stub.log' "$D\ae_stub.log" -Force }
  if (-not (Test-Path $marker)) { throw ('After Effects did not run the script in ' + $attempts + ' attempts (no marker)') }
  $t1 = Get-Date
  while ((Get-Process AfterFX -ErrorAction SilentlyContinue) -and ((Get-Date) - $t1).TotalSeconds -lt 120) { Start-Sleep 3 }
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
