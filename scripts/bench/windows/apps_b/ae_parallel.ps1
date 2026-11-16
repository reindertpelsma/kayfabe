# After Effects exotic row: several aerender processes at once (CUDA contexts times N) on one generated project, each rendering its own
# frame range. Reuses the project builder script (ae_make.jsx) for the project, then starts 4 aerender processes.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'aeparallel'
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $js = $AE_MAKE.Replace('@MODE@', 'render').Replace('@OUT@', ($D -replace '\\', '/')).Replace('@VIDEO@', '')
  Set-Content "$D\ae_make_run.jsx" $js
  New-Item -Force -ItemType Directory "$D\frames" | Out-Null
  $R.ae_file_scripts = Enable-AeFileScripts
  $p = Start-Process "$base\AfterFX.exe" -ArgumentList @('-r', "$D\ae_make_run.jsx") -PassThru
  $t0 = Get-Date; $n = 0
  while (-not $p.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt 900) { [void](Dismiss-Dialogs 'AfterFX'); Start-Sleep 3; $n++; if ($n % 5 -eq 0) { Press-Key '{ENTER}' } elseif ($n % 5 -eq 3) { Press-Key '{ESC}' } }
  if (-not (Test-Path "$D\kf.aep")) { throw 'project not created' }
  $R.t_start = Epoch
  $procs = @()
  for ($i = 0; $i -lt 4; $i++) {
    $s = $i * 15; $e = $s + 14
    $procs += Start-Process "$base\aerender.exe" -ArgumentList @('-project', "$D\kf.aep", '-comp', 'kfcomp', '-s', $s, '-e', $e, '-output', "$D\frames\p${i}_[#####].png", '-OMtemplate', 'PNG Sequence') -RedirectStandardOutput "$D\aerender$i.log" -RedirectStandardError "$D\aerender$i.err" -PassThru -WindowStyle Hidden
  }
  $t1 = Get-Date
  while (@($procs | Where-Object { -not $_.HasExited }).Count -gt 0 -and ((Get-Date) - $t1).TotalSeconds -lt 1500) { Start-Sleep 3 }
  $R.exit_codes = @($procs | ForEach-Object { if ($_.HasExited) { $_.ExitCode } else { 'running' } })
  foreach ($q in ($procs | Where-Object { -not $_.HasExited })) { Stop-Process -Id $q.Id -Force }
  $R.frame_count = @(Get-ChildItem "$D\frames" -Filter 'p*.png').Count
  $R.outputs = @(); $f = Get-ChildItem "$D\frames" -Filter 'p*.png' | Sort-Object Name; if ($f) { $R.outputs += (File-Info $f[0].FullName); $R.outputs += (File-Info $f[-1].FullName) }
  $R.quit = 'process'
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'AfterFX' } }
Finish $R
