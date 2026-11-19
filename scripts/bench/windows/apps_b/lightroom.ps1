# Lightroom Classic: a plugin that Lightroom loads from its per-user Modules folder imports a raw photo from the data volume
# and exports a 1920-pixel JPEG through the develop pipeline; the host verifies the JPEG. The plugin and job file are removed at the end.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'lightroom'
$mods = Join-Path $env:APPDATA 'Adobe\Lightroom\Modules'
$plug = Join-Path $mods 'kf.lrdevplugin'
try {
  $raw1 = Find-Asset 'dji_fc7303_0765.dng'; $raw2 = Find-Asset 'sony_dsc_rx1_0812.arw'
  $R.source = $raw1.Name + '+' + $raw2.Name
  New-Item -Force -ItemType Directory $mods | Out-Null
  New-Item -Force -ItemType Directory $plug | Out-Null
  Set-Content (Join-Path $plug 'Info.lua') $LR_INFO -Encoding ASCII
  Set-Content (Join-Path $plug 'Init.lua') $LR_INIT -Encoding ASCII
  $res = "$D\lr_result.txt"
  New-Item -Force -ItemType Directory "$D\export" | Out-Null
  Set-Content 'C:\kfhw\lr_job.txt' ("raws=" + $raw1.FullName + "|" + $raw2.FullName + "`nout=" + "$D\export" + "`nresult=" + $res)
  Step 'launch'
  Start-Process 'C:\Program Files\Adobe\Adobe Lightroom Classic\Lightroom.exe'
  $done = Wait-For { Test-Path $res } 'Lightroom' 420
  $R.plugin_result = [string](Get-Content $res -Raw -ErrorAction SilentlyContinue)
  $R.titles = @(Win-Titles 'Lightroom' | Where-Object { $_ })
  if (-not $done) { throw 'plugin did not report (not loaded or no catalog)' }
  $R.outputs = @(); $k = 0
  foreach ($jpg in (Get-ChildItem "$D\export" -Filter *.jpg | Sort-Object Name)) { Copy-Item $jpg.FullName "$D\out$k.jpg"; $R.outputs += (File-Info "$D\out$k.jpg"); $k++ }
  $R.quit = Quit-App 'Lightroom'
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Lightroom' }
Remove-Item $plug -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item 'C:\kfhw\lr_job.txt' -Force -ErrorAction SilentlyContinue
Finish $R
