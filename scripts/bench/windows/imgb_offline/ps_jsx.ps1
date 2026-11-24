# Run one ExtendScript file in the running Photoshop through COM (DoJavaScript) and keep its return string. Variables: $JSX (guest path of
# the .jsx), $TAG. The script itself writes its own progress; this wrapper records the COM error text verbatim if Photoshop throws.
$R = Init-Run 'ps_jsx'
$R.outputs = @()
try {
  $ps = $null
  for ($i = 0; $i -lt 10 -and -not $ps; $i++) { try { $ps = New-Object -ComObject Photoshop.Application } catch { Step "com retry ${i}"; Start-Sleep 5 } }
  if (-not $ps) { throw 'Photoshop COM not available' }
  $R.version = $ps.Version
  $t0 = Get-Date
  $R.start_utc = $t0.ToUniversalTime().ToString('s') + 'Z'
  try { $R.js = $ps.DoJavaScriptFile($JSX) } catch { $R.com_error = $_.Exception.Message }
  $R.secs = [int]((Get-Date) - $t0).TotalSeconds
  $R.end_utc = (Get-Date).ToUniversalTime().ToString('s') + 'Z'
  $R.shot = Shot ('after_' + $TAG)
} catch { $R.error = $_.Exception.Message }
Finish $R
