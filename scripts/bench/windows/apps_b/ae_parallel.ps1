# After Effects exotic row: several aerender processes at once (CUDA contexts times N) on one generated project, each rendering its own
# frame range. Reuses the project builder script (ae_make.jsx) for the project, then starts 4 aerender processes.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'aeparallel'
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $js = $AE_MAKE.Replace('@MODE@', 'render').Replace('@OUT@', ($D -replace '\\', '/')).Replace('@VIDEO@', '')
  Set-Content "$D\ae_make_run.jsx" $js
  New-Item -Force -ItemType Directory "$D\frames" | Out-Null
  Run-AeScript "$D\ae_make_run.jsx" "$D\ae_make.txt" 900
  if (-not (Test-Path "$D\kf.aep")) { throw 'project not created' }
  $R.t_start = Epoch
  $procs = @()
  for ($i = 0; $i -lt 4; $i++) {
    $s = $i * 15; $e = $s + 14
    $procs += Start-Process "$base\aerender.exe" -ArgumentList @('-project', "$D\kf.aep", '-comp', 'kfcomp', '-s', $s, '-e', $e) -RedirectStandardOutput "$D\aerender$i.log" -RedirectStandardError "$D\aerender$i.err" -PassThru -WindowStyle Hidden
  }
  $t1 = Get-Date
  foreach ($q in $procs) { $left = [int](1500000 - ((Get-Date) - $t1).TotalMilliseconds); if ($left -lt 1000) { $left = 1000 }; [void](Wait-Proc $q $left) }
  $R.exit_codes = @($procs | ForEach-Object { if (-not (Get-Process -Id $_.Id -ErrorAction SilentlyContinue)) { $(try { $_.ExitCode } catch { 0 }) } else { 'running' } })
  foreach ($q in $procs) { if (Get-Process -Id $q.Id -ErrorAction SilentlyContinue) { Stop-Process -Id $q.Id -Force } }
  $R.frame_count = @(Get-ChildItem "$D\frames" | Where-Object { $_.Name -match '^f_\d+\.(png|tif)$' }).Count
  $R.outputs = @(); $f = Get-ChildItem "$D\frames" | Where-Object { $_.Name -match '^f_\d+\.(png|tif)$' } | Sort-Object Name; if ($f) { $R.outputs += (File-Info $f[0].FullName); $R.outputs += (File-Info $f[-1].FullName) }
  $R.quit = 'process'
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'AfterFX' } }
Finish $R
