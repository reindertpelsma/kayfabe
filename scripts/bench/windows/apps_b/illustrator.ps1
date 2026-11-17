# Illustrator: open the data volume's SVG (or generate art when none), add generated vector art, export a 4x PNG and a PDF, quit.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'illustrator'
$exe = 'C:\Program Files\Adobe\Adobe Illustrator 2027\Support Files\Contents\Windows\Illustrator.exe'
try {
  $DATA = Data-Root
  $svg = Find-Asset 'gradient_3d_colored.svg'
  $R.source = if ($svg) { $svg.Name } else { 'generated' }
  if (-not (Test-Path $exe)) { $exe = (Get-ChildItem 'C:\Program Files\Adobe\Adobe Illustrator 2027' -Recurse -Filter Illustrator.exe | Select-Object -First 1).FullName }
  Step 'start'
  Start-Process $exe
  Step 'started'
  $ok = Wait-For { Has-Window 'Illustrator' } 'Illustrator' 150
  if (-not $ok) { throw 'Illustrator window did not appear' }
  Step 'window'
  [void](Wait-For { $false } 'Illustrator' 15)
  Step 'com'
  $ai = $null
  for ($i = 0; $i -lt 20 -and -not $ai; $i++) { try { $ai = New-Object -ComObject Illustrator.Application } catch { [void](Dismiss-Dialogs 'Illustrator'); Start-Sleep 3 } }
  if (-not $ai) { throw 'Illustrator COM not available' }
  $R.version = $ai.Version
  $srcJs = if ($svg) { "var doc = app.open(new File('" + ($svg.FullName -replace '\\','/') + "'));" } else { "var doc = app.documents.add(DocumentColorSpace.RGB, 800, 600);" }
  $js = @"
$srcJs
var w0 = doc.width, h0 = doc.height;
var lay = doc.layers.add(); lay.name = 'kf';
for (var i = 0; i < 400; i++) {
  var x = (i * 37) % Math.max(100, w0), y = (i * 53) % Math.max(100, h0);
  var e = lay.pathItems.ellipse(-y, x, 30 + (i % 40), 30 + (i % 25));
  var c = new RGBColor(); c.red = (i * 7) % 256; c.green = (i * 13) % 256; c.blue = (i * 29) % 256;
  e.fillColor = c; e.opacity = 40 + (i % 50);
}
var n = doc.pathItems.length;
var po = new ExportOptionsPNG24(); po.horizontalScale = 400; po.verticalScale = 400; po.antiAliasing = true; po.transparency = false; po.artBoardClipping = true;
doc.exportFile(new File('$($D -replace '\\','/')/out.png'), ExportType.PNG24, po);
var pdo = new PDFSaveOptions(); pdo.compatibility = PDFCompatibility.ACROBAT7;
doc.saveAs(new File('$($D -replace '\\','/')/out.pdf'), pdo);
doc.close(SaveOptions.DONOTSAVECHANGES);
'{"w0":"' + w0 + '","h0":"' + h0 + '","paths":"' + n + '"}';
"@
  $res = $ai.DoJavaScript($js)
  $R.js = ($res | ConvertFrom-Json)
  $R.outputs = @((File-Info "$D\out.png"), (File-Info "$D\out.pdf"))
  $R.quit = Quit-App 'Illustrator' $ai
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Illustrator' }
Finish $R
