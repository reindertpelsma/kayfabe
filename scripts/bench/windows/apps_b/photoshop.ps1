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
  for ($i = 0; $i -lt 15 -and -not $ps; $i++) { try { $ps = New-Object -ComObject Photoshop.Application } catch { Step "com retry ${i}: $($_.Exception.Message.Substring(0, 60))"; [void](Dismiss-Dialogs 'Photoshop'); if ($i % 3 -eq 2) { Press-Key '{ENTER}' }; Start-Sleep 6 } }
  if (-not $ps) { throw 'Photoshop COM not available' }
  $R.version = $ps.Version
  $js = @"
app.displayDialogs = DialogModes.NO;
function sid(s){return stringIDToTypeID(s);}
var out = {}, feats = [];
function P(m) { var pf = new File('$($D -replace '\\','/')/ps_progress.txt'); pf.open('a'); pf.writeln(new Date().getTime() + ' ' + m); pf.close(); }
P('start');
var si = app.systemInformation.split(/[\r\n]+/); var keep = [];
for (var i=0;i<si.length;i++){ if (/GPU|Graphics|OpenGL|Driver|VRAM|Compute/i.test(si[i])) keep.push(si[i].replace(/^\s+|\s+$/g,'')); }
out.gpuinfo = keep.join(' || ');
var t0 = new Date().getTime();
var doc = app.open(new File('$($src -replace '\\','/')'));
out.w0 = doc.width.as('px'); out.h0 = doc.height.as('px');
P('opened');
// scale: an 8K-class canvas
doc.resizeImage(UnitValue(7680,'px'), undefined, undefined, ResampleMethod.BICUBIC);
out.w_big = doc.width.as('px'); out.h_big = doc.height.as('px'); feats.push('canvas8k');
P('resized8k');
// colour: 16 bit and a wide-gamut profile
try { doc.bitsPerChannel = BitsPerChannelType.SIXTEEN; feats.push('16bit'); } catch (e) {}
try { doc.convertProfile('Display P3', Intent.RELATIVECOLORIMETRIC, true, true); feats.push('iccP3'); } catch (e) { out.err_icc = String(e).substring(0,80); }
P('colour done');
// many layers with blend modes and opacities (patches of the photo at random places)
var modes = [BlendMode.MULTIPLY, BlendMode.SCREEN, BlendMode.OVERLAY, BlendMode.SOFTLIGHT, BlendMode.HARDLIGHT, BlendMode.COLORDODGE, BlendMode.LINEARBURN, BlendMode.DIFFERENCE, BlendMode.LUMINOSITY, BlendMode.COLORBLEND];
var used = {}, base = doc.activeLayer, n = 0;
for (var i = 0; i < 100; i++) {
  try {
    var L = base.duplicate();
    L.blendMode = modes[i % modes.length]; used[i % modes.length] = 1;
    L.opacity = 20 + (i * 7) % 60;
    L.translate(UnitValue((i * 311) % 2000 - 1000, 'px'), UnitValue((i * 173) % 1200 - 600, 'px'));
    n++;
  } catch (e) { out.err_layer = String(e).substring(0,80); break; }
}
out.layers = doc.layers.length; out.blend_modes = 0; for (var k in used) out.blend_modes++;
if (n >= 100) feats.push('layers100');
P('layers done');
// adjustment layers
try {
  var ad = 0;
  var kinds = ['hueSaturation', 'curves', 'levels', 'vibrance', 'photoFilter'];
  for (var q = 0; q < kinds.length; q++) {
    try { var d = new ActionDescriptor(); var r = new ActionReference(); r.putClass(sid('adjustmentLayer')); d.putReference(sid('null'), r);
      var d2 = new ActionDescriptor(); var d3 = new ActionDescriptor(); d2.putObject(sid('type'), sid(kinds[q]), d3); d.putObject(sid('using'), sid('adjustmentLayer'), d2);
      executeAction(sid('make'), d, DialogModes.NO); ad++; } catch (e) {}
  }
  out.adjustments = ad; if (ad >= 3) feats.push('adjustment_layers');
} catch (e) {}
P('adjustments done');
// flatten a copy into one raster layer for the filters (merged visible), smart object + smart filter
try { var mv = new ActionDescriptor(); executeAction(sid('mergeVisible'), mv, DialogModes.NO); } catch (e) { try { doc.mergeVisibleLayers(); } catch (e2) {} }
try { executeAction(sid('newPlacedLayer'), new ActionDescriptor(), DialogModes.NO); feats.push('smart_object'); } catch (e) {}
P('smartobject done');
var done = 0;
for (var pass = 0; pass < 6; pass++) {
  try { var s1 = new ActionDescriptor();
    s1.putEnumerated(sid('presetKind'), sid('presetKindType'), sid('presetKindCustom'));
    s1.putUnitDouble(sid('amount'), sid('percentUnit'), 200); s1.putUnitDouble(sid('radius'), sid('pixelsUnit'), 6);
    s1.putUnitDouble(sid('noiseReduction'), sid('percentUnit'), 20); s1.putBoolean(sid('useLegacy'), false);
    s1.putEnumerated(sid('blurType'), sid('blurType'), sid('gaussianBlur'));
    executeAction(sid('smartSharpen'), s1, DialogModes.NO); done++; } catch (e) { out.err_smart = String(e).substring(0,80); }
  try { doc.activeLayer.applyGaussianBlur(2 + pass); } catch (e) {}
}
out.smart_sharpen = done; if (done >= 3) feats.push('smart_sharpen'); out.filters = new Array(done + 1).join('smartSharpen,');
P('filters done');
// brush strokes: large soft strokes on a new layer
try {
  var bl = doc.artLayers.add(); bl.name = 'strokes';
  var col = new SolidColor(); col.rgb.red = 200; col.rgb.green = 80; col.rgb.blue = 30; app.foregroundColor = col;
  var bs = 0;
  for (var j = 0; j < 40; j++) {
    var sub = new SubPathInfo(); sub.closed = false; sub.operation = ShapeOperation.SHAPEXOR;
    var p1 = new PathPointInfo(); p1.kind = PointKind.CORNERPOINT; p1.anchor = [(j * 191) % 7000, (j * 97) % 4000]; p1.leftDirection = p1.anchor; p1.rightDirection = p1.anchor;
    var p2 = new PathPointInfo(); p2.kind = PointKind.CORNERPOINT; p2.anchor = [(j * 191) % 7000 + 600, (j * 97) % 4000 + 300]; p2.leftDirection = p2.anchor; p2.rightDirection = p2.anchor;
    sub.entireSubPath = [p1, p2];
    var pth = doc.pathItems.add('kf' + j, [sub]); pth.strokePath(ToolType.BRUSH, false); pth.remove(); bs++;
  }
  out.strokes = bs; if (bs >= 20) feats.push('brush_strokes');
} catch (e) { out.err_brush = String(e).substring(0,80); }
P('strokes done');
out.features = feats.join(',');
P('flatten'); doc.flatten();
doc.resizeImage(UnitValue(2048,'px'), undefined, undefined, ResampleMethod.BICUBICSHARPER);
try { doc.bitsPerChannel = BitsPerChannelType.EIGHT; } catch (e) {}
out.w1 = doc.width.as('px'); out.h1 = doc.height.as('px');
var po = new PNGSaveOptions(); po.compression = 6; po.interlaced = false;
doc.saveAs(new File('$($D -replace '\\','/')/out.png'), po, true, Extension.LOWERCASE);
var pd = new PhotoshopSaveOptions(); pd.layers = false;
doc.saveAs(new File('$($D -replace '\\','/')/out.psd'), pd, true, Extension.LOWERCASE);
doc.close(SaveOptions.DONOTSAVECHANGES);
out.ms = new Date().getTime() - t0;
var s = '{'; for (var k2 in out) { s += '"'+k2+'":"'+String(out[k2]).replace(/\\/g,'/').replace(/"/g,"'")+'",'; } s += '"end":"1"}';
s;
"@
  $res = $ps.DoJavaScript($js)
  $R.js = ($res | ConvertFrom-Json)
  $R.outputs = @((File-Info "$D\out.png"), (File-Info "$D\out.psd"))
  $R.quit = Quit-App 'Photoshop' $ps
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Photoshop' }
Finish $R
