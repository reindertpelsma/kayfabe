# Launch-and-main-window check for the applications that have no scripting route usable offline: start the app, handle first-run
# dialogs, wait for the main window through UI Automation, hold it for a while, record the process state and the app's own GPU
# sniffer output (when it ships one), then close it cleanly. $APPNAME is set by the caller (header line).
. 'C:\kfhw\lib.ps1'
$T = @{
  premiere     = @{ exe = 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\Adobe Premiere Pro.exe'; proc = 'Adobe Premiere Pro'; title = 'Premiere'; sniffer = 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\GPUSniffer.exe' }
  mediaencoder = @{ exe = 'C:\Program Files\Adobe\Adobe Media Encoder 2027\Adobe Media Encoder.exe'; proc = 'Adobe Media Encoder'; title = 'Media Encoder'; sniffer = 'C:\Program Files\Adobe\Adobe Media Encoder 2027\GPUSniffer.exe' }
  lightroom    = @{ exe = 'C:\Program Files\Adobe\Adobe Lightroom Classic\Lightroom.exe'; proc = 'Lightroom'; title = 'Lightroom'; sniffer = $null }
  dimension    = @{ exe = 'C:\Program Files\Adobe\Adobe Dimension\Adobe Dimension.exe'; proc = 'Adobe Dimension'; title = 'Dimension'; sniffer = 'C:\Program Files\Adobe\Adobe Dimension\gpu-sniffer-core.exe' }
  uxp          = @{ exe = 'C:\Program Files\Adobe\Adobe UXP Developer Tools\Adobe UXP Developer Tools.exe'; proc = 'Adobe UXP Developer Tools'; title = 'UXP'; sniffer = $null }
}
$R = Init-Run $APPNAME
$c = $T[$APPNAME]
try {
  if ($c.sniffer -and (Test-Path $c.sniffer)) {
    ((& $c.sniffer 2>&1) | Out-String) | Set-Content -Encoding UTF8 "$D\sniffer.txt"
  }
  Step 'launch'
  Start-Process $c.exe
  $ok = Wait-For { Has-Window $c.proc } $c.proc 240
  $R.main_window = $ok
  Step "window=$ok"
  [void](Wait-For { $false } $c.proc 30)
  $R.titles = @(Win-Titles $c.proc | Where-Object { $_ })
  $pr = Get-Process $c.proc -ErrorAction SilentlyContinue | Select-Object -First 1
  $R.alive = [bool]$pr
  $R.responding = if ($pr) { [bool]$pr.Responding } else { $false }
  $R.private_mb = if ($pr) { [int]($pr.PrivateMemorySize64 / 1MB) } else { 0 }
  $R.outputs = @(); if (Test-Path "$D\sniffer.txt") { $R.outputs += (File-Info "$D\sniffer.txt") }
  $R.quit = Quit-App $c.proc
  if (-not $ok) { throw 'main window did not appear' }
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App $c.proc } }
Finish $R
