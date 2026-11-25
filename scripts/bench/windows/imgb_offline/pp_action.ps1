# Premiere Pro action A of traces/premiere_bar1_20261125/PREREGISTRATION.md, run by adobe_part2/drive.py (lib.ps1 is prepended): the Startup stub runs pp_sed.jsx
# in its `nosed` mode (new project, import the data-volume 1080p clip, sequence from the clip, active, selected, hold 45 s, done marker).
# Unlike lib.ps1 Run-StartupScript this loop sees the process end: an attempt is CRASH when Premiere is gone before the marker, PASS when the marker
# exists and the process is alive afterwards, HANG when neither within the budget. Dialogs and the ESC/ENTER cadence are those of Run-StartupScript.
# Variables: $JSXSRC (guest path of pp_sed.jsx), $TAG, $BUDGET (seconds, default 420).
$R = Init-Run 'pp_action'
$R.outputs = @()
try {
  $exe = 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\Adobe Premiere Pro.exe'; $proc = 'Adobe Premiere Pro'
  $budget = if ($BUDGET) { [int]$BUDGET } else { 420 }
  $clip = (Find-Asset 'sintel_trailer_1080p.mp4').FullName
  $js = (Get-Content $JSXSRC -Raw).Replace('@OUT@', ($D -replace '\\', '/')).Replace('@CLIP@', ($clip -replace '\\', '/')).Replace('@RENDERER@', 'nosed')
  Set-Content "$D\pp_run.jsx" $js
  New-Item -Force -ItemType Directory C:\kfhw | Out-Null
  Remove-Item C:\kfhw\pp_stub.log -ErrorAction SilentlyContinue
  Get-Process $proc -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep 3
  Set-Content 'C:\kfhw\pp_job.txt' ("$D\pp_run.jsx" -replace '\\', '/') -NoNewline
  $R.tag = $TAG
  $R.start_utc = (Get-Date).ToUniversalTime().ToString('s') + 'Z'
  $t0 = Get-Date
  $p = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe -Parent) -PassThru
  $n = 0; $holdT = $null; $outcome = $null
  while (((Get-Date) - $t0).TotalSeconds -lt $budget) {
    $prog = if (Test-Path "$D\pp_progress.txt") { [string](Get-Content "$D\pp_progress.txt" -Raw) } else { '' }
    if (-not $holdT -and $prog -match 'holding the sequence open') { $holdT = Get-Date; $R.hold_step_utc = $holdT.ToUniversalTime().ToString('s') + 'Z' }
    if (Test-Path "$D\pp_done.txt") { $outcome = 'DONE'; break }
    if (-not (Get-Process -Id $p.Id -ErrorAction SilentlyContinue)) { $outcome = 'GONE'; $R.gone_utc = (Get-Date).ToUniversalTime().ToString('s') + 'Z'; break }
    [void](Dismiss-Dialogs $proc); Start-Sleep 3; $n++
    if ($n % 15 -eq 0) { Press-Key '{ESC}' } elseif ($n % 15 -eq 7) { Press-Key '{ENTER}' }
  }
  Start-Sleep 6
  $q = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
  $R.alive_after = [bool]$q
  if ($q) { $R.responding_after = $q.Responding }
  if ($holdT -and $R.gone_utc) { $R.gone_after_hold_step_s = [math]::Round(([datetime]::Parse($R.gone_utc).ToUniversalTime() - $holdT.ToUniversalTime()).TotalSeconds, 1) }
  $R.outcome = if ($outcome -eq 'DONE' -and $q) { 'PASS' } elseif ($outcome -eq 'GONE' -or -not $q) { 'CRASH' } else { 'HANG' }
  $R.progress = [string[]]@(if (Test-Path "$D\pp_progress.txt") { Get-Content "$D\pp_progress.txt" | ForEach-Object { [string]$_ } })
  if (Test-Path C:\kfhw\pp_stub.log) { Copy-Item C:\kfhw\pp_stub.log "$D\pp_stub.log" -Force }
  if ($q) { $R.shot = Shot 'pp_after'; $R.quit = Quit-App $proc }
} catch { $R.error = $_.Exception.Message }
Finish $R
