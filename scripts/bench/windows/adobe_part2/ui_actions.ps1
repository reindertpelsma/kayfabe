# Runs the action lines in $ACTIONS (one per line): shot NAME | click X Y | dclick X Y | rclick X Y | drag X1 Y1 X2 Y2 | key SENDKEYS | type TEXT | wait S | focus PROC | titles
# Coordinates are desktop pixels. Everything is logged to steps.log; screenshots are PNG files in the run directory.
$R = Init-Run 'ui'
$R.outputs = @()
try {
  foreach ($line in ($ACTIONS -split "`n")) {
    $l = $line.Trim(); if (-not $l) { continue }
    $p = $l -split ' ', 2; $arg = if ($p.Count -gt 1) { $p[1] } else { '' }; $a = @($arg -split ' ')
    Step $l
    switch ($p[0]) {
      'shot'   { Step (Shot $arg) }
      'click'  { Click-At $a[0] $a[1] }
      'dclick' { DClick-At $a[0] $a[1] }
      'rclick' { RClick-At $a[0] $a[1] }
      'drag'   { Drag-To $a[0] $a[1] $a[2] $a[3] }
      'key'    { Press-Key $arg; Start-Sleep -Milliseconds 400 }
      'type'   { Press-Key (($arg -replace '([+^%~(){}\[\]])', '{$1}')); Start-Sleep -Milliseconds 300 }
      'wait'   { Start-Sleep -Seconds ([int]$arg) }
      'focus'  { Step ("focus=" + (Focus-Proc $arg)) }
      'titles' { (All-Titles) | Add-Content "$D\titles.txt" }
      default  { Step "unknown action" }
    }
  }
} catch { $R.error = $_.Exception.Message }
Finish $R
