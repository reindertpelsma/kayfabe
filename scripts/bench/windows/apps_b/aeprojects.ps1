# After Effects opens every open-licence AEP project of the data volume (open only: some reference footage that is not on the volume),
# records the item and layer counts, saves nothing.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'aeprojects'
$base = 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files'
try {
  $DATA = Data-Root
  $files = @(Get-ChildItem (Join-Path $DATA 'project') -Filter *.aep | Sort-Object Name)
  $R.count = $files.Count
  if ($files.Count -eq 0) { throw 'no project files on the data volume' }
  $list = ($files | ForEach-Object { $_.FullName -replace '\\', '/' }) -join '|'
  $jsx = @"
var files = '$list'.split('|'), rows = [];
app.beginSuppressDialogs();
for (var i = 0; i < files.length; i++) {
  var name = files[i].split('/').pop();
  try {
    app.open(new File(files[i]));
    var n = app.project.numItems, comps = 0, layers = 0;
    for (var k = 1; k <= n; k++) { var it = app.project.item(k); if (it instanceof CompItem) { comps++; layers += it.numLayers; } }
    rows.push('OK ' + name + ':I' + n + ':C' + comps + ':L' + layers);
    app.project.close(CloseOptions.DO_NOT_SAVE_CHANGES);
  } catch (e) { rows.push('ERR ' + name + ':' + String(e).substring(0, 60)); try { app.project.close(CloseOptions.DO_NOT_SAVE_CHANGES); } catch (e2) {} }
}
app.endSuppressDialogs(false);
var f = new File('$($D -replace '\\','/')/ae_open.txt'); f.open('w'); f.write(rows.join('||')); f.close();
app.quit();
"@
  Set-Content "$D\ae_open.jsx" $jsx
  Run-AeScript "$D\ae_open.jsx" "$D\ae_open.txt" 900
  if (Get-Process AfterFX -ErrorAction SilentlyContinue) { $R.quit = Quit-App 'AfterFX' } else { $R.quit = 'process' }
  $R.rows = (Get-Content "$D\ae_open.txt" -Raw -ErrorAction SilentlyContinue)
  $R.outputs = @(); if (Test-Path "$D\ae_open.txt") { $R.outputs += (File-Info "$D\ae_open.txt") }
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'AfterFX' } }
Finish $R
