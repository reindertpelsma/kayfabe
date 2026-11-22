# Gate row for one application of the sealed Adobe image: start it OFFLINE exactly as a user would, wait for its window, DO NOT dismiss anything,
# record every top-level window title, the UI Automation names that mention sign-in / account / licence, a screenshot, then (optionally) send keys
# and take a second screenshot, then close the application.
# Variables set by drive.py: $APPNAME, $EXE (full path or a pattern in $FIND under the install root $ROOT), $PROCNAME, $CWDMODE (install|system32),
# $OPENFILE (optional document handed on the command line), $KEYS (optional SendKeys after the settle), $WAITS (window wait), $SETTLE (seconds).
$R = Init-Run $APPNAME
$R.outputs = @()
try {
  if (-not $EXE -or -not (Test-Path -LiteralPath $EXE)) {
    $hit = $null
    if ($ROOT -and $FIND) { $hit = Get-ChildItem -LiteralPath $ROOT -Recurse -Depth 4 -File -Filter $FIND -ErrorAction SilentlyContinue | Select-Object -First 1 }
    if (-not $hit) { throw "exe not found ($EXE | $ROOT $FIND)" }
    $EXE = $hit.FullName
  }
  $R.exe_name = Split-Path $EXE -Leaf
  if ($OPENFILE -like 'KFDATA:*') { $OPENFILE = (Data-Root) + ($OPENFILE.Substring(7) -replace '/', '\') }
  $R.openfile = if ($OPENFILE) { Split-Path $OPENFILE -Leaf } else { '' }
  $cwd = if ($CWDMODE -eq 'system32') { 'C:\WINDOWS\system32' } else { Split-Path $EXE -Parent }
  $R.cwd = $cwd
  $before = @(All-Titles)
  Step "launch cwd=$cwd"
  $t0 = Get-Date
  $sp = @{ FilePath = $EXE; WorkingDirectory = $cwd; PassThru = $true }
  if ($OPENFILE) { $sp.ArgumentList = @('"' + $OPENFILE + '"') }
  $proc = Start-Process @sp
  $ok = $false
  while (((Get-Date) - $t0).TotalSeconds -lt [int]$WAITS) {
    if (Has-Window $PROCNAME) { $ok = $true; break }
    if (-not (Get-Process $PROCNAME -ErrorAction SilentlyContinue) -and ((Get-Date) - $t0).TotalSeconds -gt 40) { break }
    Start-Sleep 2
  }
  $R.main_window = $ok
  $R.window_secs = [int]((Get-Date) - $t0).TotalSeconds
  Step "window=$ok after $($R.window_secs)s"
  Start-Sleep -Seconds ([int]$SETTLE)
  $R.shot1 = Shot 'shot1'
  $R.titles = @(All-Titles | Where-Object { $_ } | ForEach-Object { Scrub-Text $_ })
  $pr = Get-Process $PROCNAME -ErrorAction SilentlyContinue | Select-Object -First 1
  $R.alive = [bool]$pr
  $R.responding = if ($pr) { [bool]$pr.Responding } else { $false }
  $R.private_mb = if ($pr) { [int]($pr.PrivateMemorySize64 / 1MB) } else { 0 }
  $lines = @(Capture-UiText $PROCNAME $TITLE 1500 40)
  $rx = '(?i)(sign.?in|log.?in|sign.?up|creative cloud|account|licen|subscription|trial|offline|internet|activate|expired)'
  $R.ui_lines = $lines.Count
  $R.ui_signin_hits = @($lines | ForEach-Object { Scrub-Text ($_ -replace "`t", ' | ') } | Where-Object { $_ -match $rx } | Select-Object -Unique -First 30)
  $enc = New-Object System.Text.UTF8Encoding($false)
  [System.IO.File]::WriteAllLines("$D\uitext.txt", [string[]]@($lines | ForEach-Object { Scrub-Text ($_ -replace "`t", ' | ') }), $enc)
  if ($KEYS) {
    [void](Focus-Proc $PROCNAME); Press-Key $KEYS; Start-Sleep -Seconds 12
    $R.shot2 = Shot 'shot2'
    $R.titles_after_keys = @(All-Titles | Where-Object { $_ } | ForEach-Object { Scrub-Text $_ })
  }
  $R.quit = Quit-App $PROCNAME
} catch { $R.error = $_.Exception.Message; if ($PROCNAME) { $R.quit = Quit-App $PROCNAME } }
Finish $R
