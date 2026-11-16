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
  Run-AeScript "$D\ae_make_run.jsx" "$D\ae_make.txt" 900
  if (Get-Process AfterFX -ErrorAction SilentlyContinue) { $R.make_quit = Quit-App 'AfterFX' }
  if (-not (Test-Path "$D\kf.aep")) { throw ('project not saved: ' + (Get-Content "$D\ae_make.txt" -Raw -ErrorAction SilentlyContinue)) }
  $R.make_log = (Get-Content "$D\ae_make.txt" -Raw)
  $R.t_render_start = Epoch
  $log = "$D\aerender.log"
  $ar = Run-Aerender @('-project', "$D\kf.aep", '-comp', 'kfcomp', '-reuse') $log "$D\frames" 900000
  if (Get-Process -Id $ar.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ar.Id -Force; throw 'aerender timed out' }
  $R.aerender_exit = $(try { $ar.ExitCode } catch { $null }); if ($R.aerender_exit -eq $null) { $R.aerender_exit = $(if ((Get-Content $log -Raw) -match 'Finished composition') { 0 } else { -1 }) }
  $R.t_render_end = Epoch
  $frames = @(Get-ChildItem "$D\frames" | Where-Object { $_.Extension -in '.png', '.tif' } | Sort-Object Name)
  $R.frame_count = $frames.Count
  $R.outputs = @($frames | Select-Object -First 1 | ForEach-Object { File-Info $_.FullName }) + @($frames | Select-Object -Last 1 | ForEach-Object { File-Info $_.FullName }) + @(File-Info $log) + @(File-Info "$D\kf.aep") + @(File-Info "$D\ae_make.txt")
  $R.quit = 'process'
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'AfterFX' }
Finish $R
