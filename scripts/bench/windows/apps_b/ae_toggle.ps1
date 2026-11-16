# After Effects exotic row: switch the project renderer CUDA -> software -> CUDA between three single-frame renders in one session.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'aetoggle'
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $jsx = $AE_TOGGLE.Replace('@OUT@', ($D -replace '\\', '/'))
  Set-Content "$D\ae_toggle_run.jsx" $jsx
  Run-AeScript "$D\ae_toggle_run.jsx" "$D\ae_toggle.txt" 1200
  if (Get-Process AfterFX -ErrorAction SilentlyContinue) { $R.quit = Quit-App 'AfterFX' } else { $R.quit = 'process' }
  $R.log = (Get-Content "$D\ae_toggle.txt" -Raw -ErrorAction SilentlyContinue)
  $R.outputs = @()
  foreach ($f in (Get-ChildItem $D -Filter 'toggle_*.png' | Sort-Object Name)) { $R.outputs += (File-Info $f.FullName) }
  if (Test-Path "$D\ae_toggle.txt") { $R.outputs += (File-Info "$D\ae_toggle.txt") }
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'AfterFX' } }
Finish $R
