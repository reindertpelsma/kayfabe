# After Effects: generate a project with a script, render it with aerender (the render benchmark), host verifies the frames.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'aftereffects'
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $DATA = Data-Root
  $vid = Find-Asset 'sintel_trailer_1080p.mp4'
  $R.footage = if ($vid) { $vid.Name } else { 'none' }
  $js = $AE_MAKE.Replace('@MODE@', 'render').Replace('@OUT@', ($D -replace '\\', '/')).Replace('@VIDEO@', $(if ($vid) { $vid.FullName -replace '\\', '/' } else { '' }))
  Set-Content "$D\ae_make_run.jsx" $js
  New-Item -Force -ItemType Directory "$D\frames" | Out-Null
  $p = Start-Process "$base\AfterFX.exe" -ArgumentList @('-r', "$D\ae_make_run.jsx") -PassThru
  $t0 = Get-Date
  $n = 0
  while (-not $p.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt 900) { [void](Dismiss-Dialogs 'AfterFX'); Start-Sleep 3; $n++; if ($n % 5 -eq 0) { Press-Key '{ENTER}' } elseif ($n % 5 -eq 3) { Press-Key '{ESC}' } }
  if (-not $p.HasExited) { $R.make_quit = Quit-App 'AfterFX'; throw 'After Effects did not finish creating the project' }
  if (-not (Test-Path "$D\kf.aep")) { throw ('project not saved: ' + (Get-Content "$D\ae_make.txt" -Raw -ErrorAction SilentlyContinue)) }
  $R.make_log = (Get-Content "$D\ae_make.txt" -Raw)
  $R.t_render_start = Epoch
  $log = "$D\aerender.log"
  $r = Start-Process "$base\aerender.exe" -ArgumentList @('-project', "$D\kf.aep", '-comp', 'kfcomp', '-reuse') -RedirectStandardOutput $log -RedirectStandardError "$D\aerender.err" -PassThru -WindowStyle Hidden
  $t1 = Get-Date
  while (-not $r.HasExited -and ((Get-Date) - $t1).TotalSeconds -lt 600) { [void](Dismiss-Dialogs 'aerender'); Start-Sleep 2 }
  if (-not $r.HasExited) { Stop-Process -Id $r.Id -Force; throw 'aerender timed out' }
  $R.aerender_exit = $r.ExitCode
  $R.t_render_end = Epoch
  $frames = @(Get-ChildItem "$D\frames" | Where-Object { $_.Extension -in '.png', '.tif' } | Sort-Object Name)
  $R.frame_count = $frames.Count
  $R.outputs = @($frames | Select-Object -First 1 | ForEach-Object { File-Info $_.FullName }) + @($frames | Select-Object -Last 1 | ForEach-Object { File-Info $_.FullName }) + @(File-Info $log) + @(File-Info "$D\kf.aep") + @(File-Info "$D\ae_make.txt")
  $R.quit = 'process'
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'AfterFX' }
Finish $R
