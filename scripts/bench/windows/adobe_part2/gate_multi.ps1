# Gate row, all applications at once: start every application OFFLINE as a user would (working directory = its install directory), do NOT dismiss anything,
# and watch for each main window. When an application's window has been up 25 s: bring it to the front, take a screenshot, record the top-level window titles of
# its processes. At the end: UI Automation names with sign-in / account / licence words per application, then quit them all.
# Variables: $WAVE (a list name), $MAXS (seconds to wait for windows), $CWDMODE (install|system32).
$R = Init-Run 'gate_multi'
$R.outputs = @()
$all = @{
  photoshop_acr = @{ exe = 'C:\Program Files\Adobe\Adobe Photoshop 2027\Photoshop.exe'; proc = 'Photoshop'; file = 'KFDATA:raw/dji_fc7303_0765.dng' }
  photoshop     = @{ exe = 'C:\Program Files\Adobe\Adobe Photoshop 2027\Photoshop.exe'; proc = 'Photoshop'; file = '' }
  illustrator   = @{ exe = $null; find = 'Illustrator.exe'; root = 'C:\Program Files\Adobe\Adobe Illustrator 2027'; proc = 'Illustrator'; file = '' }
  premiere      = @{ exe = 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\Adobe Premiere Pro.exe'; proc = 'Adobe Premiere Pro'; file = '' }
  aftereffects  = @{ exe = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files\AfterFX.exe'; proc = 'AfterFX'; file = '' }
  lightroom     = @{ exe = 'C:\Program Files\Adobe\Adobe Lightroom Classic\Lightroom.exe'; proc = 'Lightroom'; file = '' }
}
$names = @($WAVE -split ',')
$st = @{}
try {
  foreach ($n in $names) {
    $c = $all[$n]
    $exe = $c.exe
    if (-not $exe -or -not (Test-Path -LiteralPath $exe)) {
      $hit = Get-ChildItem -LiteralPath $c.root -Recurse -Depth 4 -File -Filter $c.find -ErrorAction SilentlyContinue | Select-Object -First 1
      if (-not $hit) { $st[$n] = @{ error = 'exe not found'; done = $true }; continue }
      $exe = $hit.FullName
    }
    $cwd = if ($CWDMODE -eq 'system32') { 'C:\WINDOWS\system32' } else { Split-Path $exe -Parent }
    $sp = @{ FilePath = $exe; WorkingDirectory = $cwd; PassThru = $true }
    if ($c.file) { $f = $c.file; if ($f -like 'KFDATA:*') { $f = (Data-Root) + ($f.Substring(7) -replace '/', '\') }; $sp.ArgumentList = @('"' + $f + '"') }
    $t0 = Get-Date
    $p = Start-Process @sp
    $st[$n] = @{ t0 = $t0; proc = $c.proc; exe_name = (Split-Path $exe -Leaf); cwd = $cwd; seen = $null; shot = $false; done = $false }
    Step "started $n"
    Start-Sleep -Seconds 6
  }
  $limit = (Get-Date).AddSeconds([int]$MAXS)
  while ((Get-Date) -lt $limit) {
    $pending = 0
    foreach ($n in $names) {
      $s = $st[$n]
      if ($s.done) { continue }
      $pending++
      if (-not $s.seen) {
        if (Has-Window $s.proc) { $s.seen = Get-Date; $s.window_secs = [int](($s.seen) - $s.t0).TotalSeconds; Step "$n window after $($s.window_secs)s" }
        elseif (-not (Get-Process $s.proc -ErrorAction SilentlyContinue) -and ((Get-Date) - $s.t0).TotalSeconds -gt 60) { $s.gone = $true; $s.done = $true; Step "$n process gone"; }
      } elseif (-not $s.shot -and ((Get-Date) - $s.seen).TotalSeconds -ge 25) {
        [void](Focus-Proc $s.proc); Start-Sleep 2
        $s.shot_info = Shot ("shot_" + $n); $s.shot = $true; $s.done = $true
        $s.titles = @(All-Titles | Where-Object { $_ } | ForEach-Object { Scrub-Text $_ })
        $pr = Get-Process $s.proc -ErrorAction SilentlyContinue | Select-Object -First 1
        $s.alive = [bool]$pr; $s.responding = if ($pr) { [bool]$pr.Responding } else { $false }; $s.private_mb = if ($pr) { [int]($pr.PrivateMemorySize64 / 1MB) } else { 0 }
        Step "$n captured"
      }
    }
    if ($pending -eq 0) { break }
    Start-Sleep -Seconds 5
  }
  $R.final_shot = Shot 'final'
  $R.final_titles = @(All-Titles | Where-Object { $_ } | ForEach-Object { Scrub-Text $_ })
  $rx = '(?i)(sign.?in|log.?in|sign.?up|creative cloud|account|licen|subscription|trial|offline|internet|activate|expired)'
  foreach ($n in $names) {
    $s = $st[$n]
    if ($s.seen -and -not $s.skip_ui) {
      $lines = @(Capture-UiText $s.proc $n 800 20)
      $s.ui_lines = $lines.Count
      $s.ui_signin_hits = @($lines | ForEach-Object { Scrub-Text ($_ -replace "`t", ' | ') } | Where-Object { $_ -match $rx } | Select-Object -Unique -First 20)
    }
  }
} catch { $R.error = $_.Exception.Message }
$R.apps = @{}
foreach ($n in $names) {
  $s = $st[$n]; $o = [ordered]@{}
  foreach ($k in @('exe_name', 'cwd', 'window_secs', 'gone', 'alive', 'responding', 'private_mb', 'shot_info', 'titles', 'ui_lines', 'ui_signin_hits', 'error')) { if ($s.ContainsKey($k)) { $o[$k] = $s[$k] } }
  $o.main_window = [bool]$s.seen
  $R.apps[$n] = $o
}
foreach ($n in $names) { try { $R.apps[$n].quit = Quit-App $st[$n].proc $null 60 } catch {} }
Finish $R
