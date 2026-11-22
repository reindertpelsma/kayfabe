# Part 2 (a): Substance 3D Painter started twice in ONE boot, only the working directory differs: C:\WINDOWS\system32 (what a scheduled task / the older launch.ps1 gave it),
# then its install directory. Per start: wait for a main window (or for the process to end), hold 45 s, record alive/responding, the top-level window titles, a screenshot,
# and the Application Error / WER events for the process since the start; then kill it. Variables: $ORDER (e.g. system32,install), $WAITS (window wait per start, seconds).
$R = Init-Run 'painter_cwd'
$R.outputs = @()
$R.runs = @()
$exe = (Get-ChildItem -LiteralPath 'C:\Program Files\Adobe\Adobe Substance 3D Painter' -File -Filter 'Adobe Substance 3D Painter.exe' -ErrorAction SilentlyContinue | Select-Object -First 1).FullName
try {
  if (-not $exe) { throw 'Painter exe not found' }
  $R.exe_dir = Split-Path (Split-Path $exe -Parent) -Leaf
  $R.version = (Get-Item $exe).VersionInfo.FileVersion
  foreach ($mode in @($ORDER -split ',')) {
    $cwd = if ($mode -eq 'system32') { 'C:\WINDOWS\system32' } else { Split-Path $exe -Parent }
    Get-Process 'Adobe Substance 3D Painter' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep 3
    $t0 = Get-Date
    $o = [ordered]@{ mode = $mode; cwd = $cwd }
    Step "start mode=$mode cwd=$cwd"
    $p = Start-Process -FilePath $exe -WorkingDirectory $cwd -PassThru
    $win = $false; $ended = $false
    while (((Get-Date) - $t0).TotalSeconds -lt [int]$WAITS) {
      if (-not (Get-Process -Id $p.Id -ErrorAction SilentlyContinue)) { $ended = $true; break }
      if (Has-Window 'Adobe Substance 3D Painter') { $win = $true; break }
      Start-Sleep 3
    }
    $o.window = $win; $o.process_ended_before_window = $ended; $o.secs_to_window_or_end = [int]((Get-Date) - $t0).TotalSeconds
    if ($win) { Start-Sleep 45 }
    $alive = [bool](Get-Process 'Adobe Substance 3D Painter' -ErrorAction SilentlyContinue)
    $o.alive_after_hold = $alive
    $pr = Get-Process 'Adobe Substance 3D Painter' -ErrorAction SilentlyContinue | Select-Object -First 1
    $o.responding = if ($pr) { [bool]$pr.Responding } else { $false }
    $o.titles = @(All-Titles | Where-Object { $_ } | ForEach-Object { Scrub-Text $_ })
    $o.shot = Shot ("painter_" + $mode)
    $ev = @()
    foreach ($e in @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $t0 } -ErrorAction SilentlyContinue | Where-Object { $_.Id -in 1000, 1001 -or $_.ProviderName -match 'Application Error|Windows Error Reporting' } | Select-Object -First 6)) {
      $m = ($e.Message -replace '\s+', ' ')
      if ($m.Length -gt 230) { $m = $m.Substring(0, 230) }
      $ev += ('{0} id={1} {2}' -f $e.TimeCreated.ToString('HH:mm:ss'), $e.Id, $m)
    }
    $o.app_events = $ev
    $o.crash_qt6webengine = [bool](@($ev | Where-Object { $_ -match 'Qt6WebEngineCore' }).Count)
    $R.runs += $o
    Get-Process 'Adobe Substance 3D Painter' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep 5
  }
} catch { $R.error = $_.Exception.Message }
Finish $R
