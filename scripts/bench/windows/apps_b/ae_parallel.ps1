# After Effects exotic row: several aerender processes at once (CUDA contexts times N) on one generated project, each rendering its own
# frame range. Reuses the project builder script (ae_make.jsx) for the project, then starts 4 aerender processes.
. 'C:\kfhw\lib.ps1'
$R = Init-Run $APPNAME
$np = if ($APPNAME -eq 'aeparallel4') { 4 } else { 2 }
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $js = $AE_MAKE.Replace('@MODE@', 'render').Replace('@OUT@', ($D -replace '\\', '/')).Replace('@VIDEO@', '')
  Set-Content "$D\ae_make_run.jsx" $js
  New-Item -Force -ItemType Directory "$D\frames" | Out-Null
  Run-AeScript "$D\ae_make_run.jsx" "$D\ae_make.txt" 900
  if (Get-Process AfterFX -ErrorAction SilentlyContinue) { $R.make_quit = Quit-App 'AfterFX' }
  if (-not (Test-Path "$D\kf.aep")) { throw 'project not created' }
  $R.t_start = Epoch
  $procs = @()
  for ($i = 0; $i -lt $np; $i++) {
    $per = 60 / $np; $s = $i * $per; $e = $s + $per - 1
    $procs += Start-Process "$base\aerender.exe" -ArgumentList @('-project', "$D\kf.aep", '-comp', 'kfcomp', '-s', $s, '-e', $e) -RedirectStandardOutput "$D\aerender$i.log" -RedirectStandardError "$D\aerender$i.err" -PassThru -WindowStyle Hidden
  }
  $t1 = Get-Date
  foreach ($q in $procs) { $left = [int](1500000 - ((Get-Date) - $t1).TotalMilliseconds); if ($left -lt 1000) { $left = 1000 }; [void](Wait-Proc $q $left) }
  $ix = 0; $R.exit_codes = @($procs | ForEach-Object { $c = $null; if (-not (Get-Process -Id $_.Id -ErrorAction SilentlyContinue)) { try { $c = $_.ExitCode } catch { } ; if ($c -eq $null) { $c = $(if ((Test-Path "$D\aerender$ix.log") -and ([string](Get-Content "$D\aerender$ix.log" -Raw)) -match 'Finished composition') { 0 } else { -1 }) } } else { $c = 'running' }; $ix++; $c })
  foreach ($q in $procs) { if (Get-Process -Id $q.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $q.Id -Force } }
  $R.frame_count = @(Get-ChildItem "$D\frames" | Where-Object { $_.Name -match '^f_\d+\.(png|tif)$' }).Count
  $R.outputs = @(); $f = Get-ChildItem "$D\frames" | Where-Object { $_.Name -match '^f_\d+\.(png|tif)$' } | Sort-Object Name; if ($f) { $R.outputs += (Copy-Out $f[0]); $R.outputs += (Copy-Out $f[-1]) }
  $R.quit = 'process'
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'AfterFX' } }
Finish $R
