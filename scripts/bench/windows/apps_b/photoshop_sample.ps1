# Photoshop on the image's own sample files (a sky preset photo and a document-template PSD shipped with the applications): open both,
# run GPU-class filters on the photo, save it as PNG, and report the template's size. Nothing is copied off the image.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'photoshopsample'
try {
  $sky = Get-ChildItem 'C:\Program Files\Adobe\Adobe Photoshop 2027\Required\Sky_Presets' -Filter *.jpg | Sort-Object Length -Descending | Select-Object -First 1
  $psd = Get-ChildItem 'C:\Program Files\Adobe\Adobe Media Encoder 2027' -Recurse -Filter '3840x2160x1.0.psd' -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $sky -or -not $psd) { throw 'shipped sample files not found' }
  $R.source = 'sky preset + 3840x2160 template'; $R.source_bytes = $sky.Length
  Start-Process 'C:\Program Files\Adobe\Adobe Photoshop 2027\Photoshop.exe'
  [void](Wait-For { Has-Window 'Photoshop' } 'Photoshop' 150)
  [void](Wait-For { $false } 'Photoshop' 20)
  $ps = $null
  for ($i = 0; $i -lt 15 -and -not $ps; $i++) { try { $ps = New-Object -ComObject Photoshop.Application } catch { [void](Dismiss-Dialogs 'Photoshop'); if ($i % 3 -eq 2) { Press-Key '{ENTER}' }; Start-Sleep 6 } }
  if (-not $ps) { throw 'Photoshop COM not available' }
  $js = @"
app.displayDialogs = DialogModes.NO;
function sid(s){return stringIDToTypeID(s);}
var t = app.open(new File('$($psd.FullName -replace '\\','/')'));
var tw = t.width.as('px'), th = t.height.as('px'), tl = t.layers.length;
t.close(SaveOptions.DONOTSAVECHANGES);
var doc = app.open(new File('$($sky.FullName -replace '\\','/')'));
var w0 = doc.width.as('px'), h0 = doc.height.as('px'), done = 0;
for (var pass = 0; pass < 4; pass++) {
  try { var d = new ActionDescriptor();
    d.putEnumerated(sid('presetKind'), sid('presetKindType'), sid('presetKindCustom'));
    d.putUnitDouble(sid('amount'), sid('percentUnit'), 150); d.putUnitDouble(sid('radius'), sid('pixelsUnit'), 4);
    d.putUnitDouble(sid('noiseReduction'), sid('percentUnit'), 20); d.putBoolean(sid('useLegacy'), false);
    d.putEnumerated(sid('blurType'), sid('blurType'), sid('gaussianBlur'));
    executeAction(sid('smartSharpen'), d, DialogModes.NO); done++; } catch (e) {}
}
doc.resizeImage(UnitValue(1920,'px'), undefined, undefined, ResampleMethod.BICUBICSHARPER);
var w1 = doc.width.as('px'), h1 = doc.height.as('px');
var po = new PNGSaveOptions(); po.compression = 6;
doc.saveAs(new File('$($D -replace '\\','/')/out.png'), po, true, Extension.LOWERCASE);
doc.close(SaveOptions.DONOTSAVECHANGES);
'{"tw":"' + tw + '","th":"' + th + '","tl":"' + tl + '","w0":"' + w0 + '","h0":"' + h0 + '","w1":"' + w1 + '","h1":"' + h1 + '","sharpen":"' + done + '"}';
"@
  $R.js = ($ps.DoJavaScript($js) | ConvertFrom-Json)
  $R.outputs = @(File-Info "$D\out.png")
  $R.quit = Quit-App 'Photoshop' $ps
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Photoshop' }
Finish $R
