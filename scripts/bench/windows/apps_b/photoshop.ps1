# Photoshop: open the data volume's photo, run GPU-accelerated filters, save PNG and PSD, quit. Host verifies the outputs.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'photoshop'
$exe = 'C:\Program Files\Adobe\Adobe Photoshop 2027\Photoshop.exe'
try {
  $DATA = Data-Root
  $src = (Find-Asset 'ktiponissi_dirfys_4480x2928.jpg').FullName
  $R.source = Split-Path $src -Leaf
  Start-Process $exe
  $ok = Wait-For { @(Win-Titles 'Photoshop') -contains 'Adobe Photoshop 2027' } 'Photoshop' 120
  if (-not $ok) { throw 'Photoshop main window did not appear' }
  [void](Wait-For { $false } 'Photoshop' 20)       # first-run and graphics-notice dialogs
  $ps = $null
  for ($i = 0; $i -lt 15 -and -not $ps; $i++) { try { $ps = New-Object -ComObject Photoshop.Application } catch { Step "com retry ${i}: $($_.Exception.Message.Substring(0, 60))"; [void](Dismiss-Dialogs 'Photoshop'); Start-Sleep 6 } }
  if (-not $ps) { throw 'Photoshop COM not available' }
  $R.version = $ps.Version
  $js = @"
app.displayDialogs = DialogModes.NO;
function sid(s){return stringIDToTypeID(s);}
var out = {};
var si = app.systemInformation.split(/[\r\n]+/); var keep = [];
for (var i=0;i<si.length;i++){ if (/GPU|Graphics|OpenGL|Driver|VRAM|Compute/i.test(si[i])) keep.push(si[i].replace(/^\s+|\s+$/g,'')); }
out.gpuinfo = keep.join(' || ');
var doc = app.open(new File('$($src -replace '\\','/')'));
out.w0 = doc.width.as('px'); out.h0 = doc.height.as('px');
var done = [];
for (var pass=0; pass<6; pass++) {
  try { var d = new ActionDescriptor();
    d.putEnumerated(sid('presetKind'), sid('presetKindType'), sid('presetKindCustom'));
    d.putUnitDouble(sid('amount'), sid('percentUnit'), 200); d.putUnitDouble(sid('radius'), sid('pixelsUnit'), 6);
    d.putUnitDouble(sid('noiseReduction'), sid('percentUnit'), 20); d.putBoolean(sid('useLegacy'), false);
    d.putEnumerated(sid('blurType'), sid('blurType'), sid('gaussianBlur'));
    executeAction(sid('smartSharpen'), d, DialogModes.NO); done.push('smartSharpen'); } catch (e) { out.err_smart = String(e).substring(0,120); }
  try { doc.activeLayer.applyGaussianBlur(3 + pass); done.push('gaussianBlur'); } catch (e) { out.err_blur = String(e).substring(0,120); }
}
out.filters = done.join(',');
doc.resizeImage(UnitValue(2048,'px'), undefined, undefined, ResampleMethod.BICUBICSHARPER);
out.w1 = doc.width.as('px'); out.h1 = doc.height.as('px');
var po = new PNGSaveOptions(); po.compression = 6; po.interlaced = false;
doc.saveAs(new File('$($D -replace '\\','/')/out.png'), po, true, Extension.LOWERCASE);
var pd = new PhotoshopSaveOptions(); pd.layers = true;
doc.saveAs(new File('$($D -replace '\\','/')/out.psd'), pd, true, Extension.LOWERCASE);
doc.close(SaveOptions.DONOTSAVECHANGES);
var s = '{'; for (var k in out) { s += '"'+k+'":"'+String(out[k]).replace(/\\/g,'/').replace(/"/g,"'")+'",'; } s += '"end":"1"}';
s;
"@
  $res = $ps.DoJavaScript($js)
  $R.js = ($res | ConvertFrom-Json)
  $R.outputs = @((File-Info "$D\out.png"), (File-Info "$D\out.psd"))
  $R.quit = Quit-App 'Photoshop' $ps
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Photoshop' }
Finish $R
