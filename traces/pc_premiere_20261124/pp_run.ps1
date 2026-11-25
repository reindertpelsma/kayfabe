# pp_run.ps1 -Clip NAME -Mode hold|probe|software -Tag T -Hold S : one attempt of action A (PREREGISTRATION.md), run in the console user's
# session at the normal integrity level. Premiere is started with its install folder as the working directory; pp.jsx is handed over by the
# Startup stub (C:\kfhw\pp_job.txt, the PR #588 mechanism). Dialogs are dismissed (safe buttons only) until the sequence step starts, never after,
# and no key is sent, so nothing but the script acts on the sequence. With -Keys the wait copies PR #588's lib.ps1 Run-StartupScript instead:
# dialogs are swept for the whole wait and the foreground window gets ESC at every 15th poll and ENTER at every 15th poll offset 7 (poll = sweep
# + 3 s), each key logged with its time. Writes C:\kfhw\pp\<Tag>\result.json.
param([string]$Clip, [string]$Mode = 'hold', [string]$Tag, [int]$Hold = 45, [switch]$Keys)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
$exe = 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\Adobe Premiere Pro.exe'
$proc = 'Adobe Premiere Pro'
$D = "C:\kfhw\pp\$Tag"
New-Item -Force -ItemType Directory $D | Out-Null
$R = [ordered]@{ tag = $Tag; clip = $Clip; mode = $Mode; hold = $Hold; keys_mode = [bool]$Keys; keys = @(); dismissed = @() }
$Safe = @('OK', 'Ok', 'Continue', 'Not Now', 'Not now', 'Skip', 'Got it', 'Got It', 'Maybe Later', 'No Thanks', 'No thanks', 'Done', 'Later',
  'Remind Me Later', "Don't Save", 'Do not show again', 'Close', 'Dismiss')
function Dismiss {
  $pids = @(Get-Process $proc -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  $roots = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
  foreach ($r in $roots) {
    try { if ($pids -notcontains $r.Current.ProcessId) { continue } } catch { continue }
    $level = @($r)
    try { $level += @($r.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' }) } catch {}
    foreach ($w in $level) {
      try { $kids = $w.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition) } catch { continue }
      foreach ($el in $kids) {
        if ($el.Current.ControlType.ProgrammaticName -ne 'ControlType.Button') { continue }
        $nm = $el.Current.Name
        if ($Safe -contains $nm -and $el.Current.IsEnabled) {
          try { $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); $script:R.dismissed += ('{0}: {1}' -f $w.Current.Name, $nm) } catch {}
        }
      }
    }
  }
}
function Utc { (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ') }
try {
  $cd = Get-Volume | Where-Object { $_.DriveType -eq 'CD-ROM' -and $_.DriveLetter } | ForEach-Object { $_.DriveLetter + ':\' }
  $f = $null
  if ($Mode -eq 'learn') { $f = Get-ChildItem "C:\ProgramData\Documents\Adobe\Premiere Pro 2027\Learn Panel" -Recurse -Force -Filter $Clip -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch 'Auto-Save' } | Select-Object -First 1; $cd = @() }
  foreach ($c in $cd) { $f = Get-ChildItem $c -Recurse -Filter $Clip -ErrorAction SilentlyContinue | Select-Object -First 1; if ($f) { break } }
  if (-not $f) { throw "clip $Clip not found" }
  $R.clip_bytes = $f.Length
  $js = (Get-Content 'C:\kfhw\pp\pp.jsx' -Raw).Replace('@OUT@', ($D -replace '\\', '/')).Replace('@CLIP@', ($f.FullName -replace '\\', '/')).Replace('@MODE@', $Mode).Replace('@HOLD@', [string]$Hold)
  Set-Content "$D\pp_run.jsx" $js
  Remove-Item "$D\pp_done.txt", "$D\pp_progress.txt" -ErrorAction SilentlyContinue
  Get-Process $proc -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep 3
  Set-Content 'C:\kfhw\pp_job.txt' ("$D\pp_run.jsx" -replace '\\', '/') -NoNewline
  $R.start_utc = Utc
  $sp = @{ FilePath = $exe; WorkingDirectory = (Split-Path $exe -Parent); PassThru = $true }
  $p = Start-Process @sp
  $R.pid = $p.Id
  $t0 = Get-Date; $seqT = $null; $goneT = $null; $n = 0
  while (((Get-Date) - $t0).TotalSeconds -lt 900) {
    $prog = if (Test-Path "$D\pp_progress.txt") { Get-Content "$D\pp_progress.txt" -Raw } else { '' }
    if (-not $seqT -and $prog -match 'creating sequence') { $seqT = Get-Date; $R.sequence_step_utc = Utc }
    if ($Mode -ne 'learn' -and (Test-Path "$D\pp_done.txt")) { $R.done_utc = Utc; break }
    if (-not (Get-Process -Id $p.Id -ErrorAction SilentlyContinue)) { $goneT = Get-Date; $R.process_gone_utc = Utc; break }
    if ($Mode -eq 'learn' -and $seqT -and ((Get-Date) - $seqT).TotalSeconds -gt $Hold) { $R.done_utc = Utc; break }
    if ($seqT -and ((Get-Date) - $seqT).TotalSeconds -gt ($Hold + 300)) { $R.hang = $true; break }
    if ($Keys) {
      Dismiss; Start-Sleep 3; $n++
      if ($n % 15 -eq 0) { [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); $R.keys += ('ESC ' + (Utc)) }
      elseif ($n % 15 -eq 7) { [System.Windows.Forms.SendKeys]::SendWait('{ENTER}'); $R.keys += ('ENTER ' + (Utc)) }
      continue
    }
    if (-not $seqT) { Dismiss }
    Start-Sleep 1
  }
  Start-Sleep 8
  $q = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
  $R.alive_after = [bool]$q
  if ($q) { $R.responding_after = $q.Responding; $R.cpu_s = [int]$q.CPU; $R.ws_mb = [int]($q.WorkingSet64 / 1MB) }
  if ($seqT -and $goneT) { $R.gone_after_sequence_s = [math]::Round(($goneT - $seqT).TotalSeconds, 1) }
  $R.done = (Test-Path "$D\pp_done.txt") -or ($Mode -eq 'learn' -and $R.done_utc)
  if (Test-Path "$D\pp_done.txt") { $R.summary = [string]((Get-Content "$D\pp_done.txt" -Raw).Trim()) }
  $R.progress = [string[]]@(if (Test-Path "$D\pp_progress.txt") { Get-Content "$D\pp_progress.txt" | ForEach-Object { [string]$_ } })
  if ($q) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; $R.stopped_by_harness_utc = Utc }
} catch { $R.error = $_.Exception.Message }
$R.end_utc = Utc
$R | ConvertTo-Json -Depth 4 | Set-Content "$D\result.json"
$R | ConvertTo-Json -Depth 4
