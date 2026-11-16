# After Effects exotic row: switch the project renderer CUDA -> software -> CUDA between three single-frame renders in one session.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'aetoggle'
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $jsx = $AE_TOGGLE.Replace('@OUT@', ($D -replace '\\', '/'))
  Set-Content "$D\ae_toggle_run.jsx" $jsx
  $p = Start-Process "$base\AfterFX.exe" -ArgumentList @('-r', "$D\ae_toggle_run.jsx") -PassThru
  $t0 = Get-Date; $n = 0
  while (-not $p.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt 900) { [void](Dismiss-Dialogs 'AfterFX'); Start-Sleep 3; $n++; if ($n % 5 -eq 0) { Press-Key '{ENTER}' } elseif ($n % 5 -eq 3) { Press-Key '{ESC}' } }
  if (-not $p.HasExited) { $R.quit = Quit-App 'AfterFX'; throw 'After Effects did not finish the renderer switch run' } else { $R.quit = 'process' }
  $R.log = (Get-Content "$D\ae_toggle.txt" -Raw -ErrorAction SilentlyContinue)
  $R.outputs = @()
  foreach ($f in (Get-ChildItem $D -Filter 'toggle_*.png' | Sort-Object Name)) { $R.outputs += (File-Info $f.FullName) }
  if (Test-Path "$D\ae_toggle.txt") { $R.outputs += (File-Info "$D\ae_toggle.txt") }
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'AfterFX' } }
Finish $R
