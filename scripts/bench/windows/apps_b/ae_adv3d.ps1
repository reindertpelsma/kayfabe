# After Effects exotic row: Advanced 3D renderer with depth of field and 24 3D layers at 4K / 16 bit, 10 frames by aerender.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'aeadv3d'
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $js = $AE_ADV3D.Replace('@OUT@', ($D -replace '\\', '/'))
  Set-Content "$D\ae_adv3d_run.jsx" $js
  New-Item -Force -ItemType Directory "$D\frames" | Out-Null
  Run-AeScript "$D\ae_adv3d_run.jsx" "$D\ae_make.txt" 900
  if (Get-Process AfterFX -ErrorAction SilentlyContinue) { $R.make_quit = Quit-App 'AfterFX' }
  if (-not (Test-Path "$D\kf.aep")) { throw ('project not saved: ' + (Get-Content "$D\ae_make.txt" -Raw -ErrorAction SilentlyContinue)) }
  $R.make_log = (Get-Content "$D\ae_make.txt" -Raw)
  $log = "$D\aerender.log"
  $ar = Run-Aerender @('-project', "$D\kf.aep", '-comp', 'kfcomp') $log "$D\frames" 900000
  if (Get-Process -Id $ar.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $ar.Id -Force; throw 'aerender timed out' }
  $R.aerender_exit = $(try { $ar.ExitCode } catch { $null }); if ($R.aerender_exit -eq $null) { $R.aerender_exit = $(if ((Get-Content $log -Raw) -match 'Finished composition') { 0 } else { -1 }) }
  $f = Get-ChildItem "$D\frames" -Filter 'f_*.png' | Sort-Object Name
  $R.frame_count = @($f).Count
  $R.outputs = @(); if ($f) { $R.outputs += (File-Info $f[0].FullName); $R.outputs += (File-Info $f[-1].FullName) }
  if (Test-Path "$D\aerender.log") { $R.outputs += (File-Info "$D\aerender.log") }
  $R.outputs += (File-Info "$D\ae_make.txt")
  $R.quit = 'process'
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'AfterFX' } }
Finish $R
