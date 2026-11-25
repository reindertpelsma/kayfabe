# Start one application as a user would (working directory = its install directory), optionally with a file from the data volume, wait for its
# main window, hold, take a screenshot and record the top-level window titles. Nothing is dismissed. Variables: $EXE, $PROC, $FILE ('' or
# 'KFDATA:relative/path'), $MAXS (window wait), $HOLDS (seconds after the window appears).
$R = Init-Run 'app_open'
$R.outputs = @()
try {
  $sp = @{ FilePath = $EXE; WorkingDirectory = (Split-Path $EXE -Parent); PassThru = $true }
  if ($FILE) { $f = $FILE; if ($f -like 'KFDATA:*') { $f = (Data-Root) + ($f.Substring(7) -replace '/', '\') }; $sp.ArgumentList = @('"' + $f + '"'); $R.file = Split-Path $f -Leaf }
  $t0 = Get-Date
  $R.start_utc = $t0.ToUniversalTime().ToString('s') + 'Z'
  $p = Start-Process @sp
  $seen = $false
  while (((Get-Date) - $t0).TotalSeconds -lt [int]$MAXS) { if (Has-Window $PROC) { $seen = $true; break }; Start-Sleep 3 }
  $R.window_secs = [int]((Get-Date) - $t0).TotalSeconds
  $R.main_window = $seen
  Step "window=$seen after $($R.window_secs)s"
  Start-Sleep ([int]$HOLDS)
  [void](Focus-Proc $PROC); Start-Sleep 2
  $R.shot = Shot 'opened'
  $R.titles = @(All-Titles | Where-Object { $_ } | ForEach-Object { Scrub-Text $_ })
} catch { $R.error = $_.Exception.Message }
Finish $R
