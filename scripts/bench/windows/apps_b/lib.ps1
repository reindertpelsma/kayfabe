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
  $pids = Proc-Pids $procNames
  $roots = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
  $n = 0
  foreach ($r in $roots) {
    try { if ($pids -notcontains $r.Current.ProcessId) { continue } } catch { continue }
    $all = $r.FindAll([System.Windows.Automation.TreeScope]::Subtree, [System.Windows.Automation.Condition]::TrueCondition)
    $title = $r.Current.Name
    foreach ($el in $all) {
      if ($el.Current.ControlType.ProgrammaticName -ne 'ControlType.Button') { continue }
      $nm = $el.Current.Name
      $list = if ($only) { $only } else { $Script:DialogButtons }
      if ($list -contains $nm -and $Script:NeverButtons -notcontains $nm -and ($nm -ne 'Close' -or $r.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' -and $title -notmatch '^(Adobe |Photoshop|Illustrator|InDesign|Animate|Premiere|After Effects|Lightroom|Acrobat|Dimension)') -and $el.Current.IsEnabled) {
        try { $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); $n++; $Script:Dismissed += ("{0}: {1}" -f $title, $nm) } catch {}
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
function Has-Window($procNames) { (@(Win-Titles $procNames | Where-Object { $_ })).Count -gt 0 }
function Quit-App($procNames, $comApp = $null) {
  # clean exit: ask politely first, then close the windows, only then end the process (and say which one happened)
  if ($comApp) { try { $comApp.Quit() } catch { try { $comApp.Exit() } catch {} } }
  $t0 = Get-Date
  while (((Get-Date) - $t0).TotalSeconds -lt 150) {
    if (-not (Proc-Pids $procNames)) { return 'clean' }
    foreach ($p in Get-Process $procNames -ErrorAction SilentlyContinue) { [void]$p.CloseMainWindow() }
    [void](Dismiss-Dialogs $procNames @("Don't Save", 'No', 'Discard', 'OK'))
    Start-Sleep 2
  }
  foreach ($p in Get-Process $procNames -ErrorAction SilentlyContinue) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  return 'forced'
}
function Finish($R) {
  $R.ended = Epoch
  $R.dismissed = @($Script:Dismissed)
  ($R | ConvertTo-Json -Depth 6) | Set-Content -Encoding UTF8 "$D\result.json"
}
function Init-Run($app) {
  New-Item -Force -ItemType Directory $D | Out-Null
  return [ordered]@{ app = $app; started = (Epoch); error = $null; outputs = @() }
}
function File-Info($path) { $f = Get-Item $path; return [ordered]@{ file = $f.Name; bytes = $f.Length; modified = ([DateTimeOffset]$f.LastWriteTimeUtc).ToUnixTimeMilliseconds() / 1000.0 } }
