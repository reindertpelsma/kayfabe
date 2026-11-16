# Photoshop opens every open-licence PSD project of the data volume (project/*.psd), records size, layer count and colour mode per file,
# and saves a 1024-pixel PNG of the flattened result of up to 6 of them. One file failing does not stop the others.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'photoshopprojects'
try {
  $DATA = Data-Root
  $files = @(Get-ChildItem (Join-Path $DATA 'project') -Filter *.psd | Sort-Object Name)
  $R.count = $files.Count
  if ($files.Count -eq 0) { throw 'no project files on the data volume' }
  Start-Process 'C:\Program Files\Adobe\Adobe Photoshop 2027\Photoshop.exe'
  [void](Wait-For { Has-Window 'Photoshop' } 'Photoshop' 150)
  [void](Wait-For { $false } 'Photoshop' 20)
  $ps = $null
  for ($i = 0; $i -lt 15 -and -not $ps; $i++) { try { $ps = New-Object -ComObject Photoshop.Application } catch { [void](Dismiss-Dialogs 'Photoshop'); if ($i % 3 -eq 2) { Press-Key '{ENTER}' }; Start-Sleep 6 } }
  if (-not $ps) { throw 'Photoshop COM not available' }
  $list = ($files | ForEach-Object { $_.FullName -replace '\\', '/' }) -join '|'
  $js = @"
app.displayDialogs = DialogModes.NO;
var files = '$list'.split('|'), rows = [], saved = 0;
for (var i = 0; i < files.length; i++) {
  var name = files[i].split('/').pop();
  try {
    var d = app.open(new File(files[i]));
    var r = name + ':' + d.width.as('px') + 'x' + d.height.as('px') + ':L' + d.layers.length + ':' + d.mode;
    if (saved < 6) {
      d.flatten(); var w = d.width.as('px'); if (w > 1024) d.resizeImage(UnitValue(1024, 'px'), undefined, undefined, ResampleMethod.BICUBIC);
      var po = new PNGSaveOptions(); d.saveAs(new File('$($D -replace '\\','/')/p' + saved + '.png'), po, true, Extension.LOWERCASE); saved++;
    }
    d.close(SaveOptions.DONOTSAVECHANGES); rows.push('OK ' + r);
  } catch (e) { rows.push('ERR ' + name + ':' + String(e).substring(0, 60)); try { app.activeDocument.close(SaveOptions.DONOTSAVECHANGES); } catch (e2) {} }
}
rows.join('||');
"@
  $R.rows = [string]($ps.DoJavaScript($js))
  $R.outputs = @(Get-ChildItem $D -Filter p*.png | ForEach-Object { File-Info $_.FullName })
  $R.quit = Quit-App 'Photoshop' $ps
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Photoshop' }
Finish $R
