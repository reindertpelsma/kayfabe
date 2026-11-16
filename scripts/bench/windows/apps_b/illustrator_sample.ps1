# Illustrator on the image's own sample art (a presets library document shipped with the application; nothing is copied off the image):
# open it, export a 2x PNG and a PDF, quit.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'illustratorsample'
try {
  $ai0 = Get-ChildItem 'C:\Program Files\Adobe\Adobe Illustrator 2027' -Recurse -Filter 'Dot Pattern Vector Pack.ai' -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $ai0) { throw 'shipped sample document not found' }
  $R.source = $ai0.Name; $R.source_bytes = $ai0.Length
  $exe = (Get-ChildItem 'C:\Program Files\Adobe\Adobe Illustrator 2027' -Recurse -Filter Illustrator.exe | Select-Object -First 1).FullName
  Start-Process $exe
  [void](Wait-For { Has-Window 'Illustrator' } 'Illustrator' 150)
  [void](Wait-For { $false } 'Illustrator' 15)
  $ai = $null
  for ($i = 0; $i -lt 20 -and -not $ai; $i++) { try { $ai = New-Object -ComObject Illustrator.Application } catch { [void](Dismiss-Dialogs 'Illustrator'); Start-Sleep 3 } }
  if (-not $ai) { throw 'Illustrator COM not available' }
  $js = @"
var doc = app.open(new File('$($ai0.FullName -replace '\\','/')'));
var w0 = doc.width, h0 = doc.height, n = doc.pathItems.length, sy = doc.symbols.length;
var sc = Math.max(10, Math.min(200, Math.floor(Math.sqrt(16000000 / (w0 * h0)) * 100)));
var po = new ExportOptionsPNG24(); po.horizontalScale = sc; po.verticalScale = sc; po.antiAliasing = true; po.transparency = false; po.artBoardClipping = true;
doc.exportFile(new File('$($D -replace '\\','/')/out.png'), ExportType.PNG24, po);
doc.saveAs(new File('$($D -replace '\\','/')/out.pdf'), new PDFSaveOptions());
doc.close(SaveOptions.DONOTSAVECHANGES);
'{"w0":"' + w0 + '","h0":"' + h0 + '","scale":"' + sc + '","paths":"' + n + '","symbols":"' + sy + '"}';
"@
  $R.js = ($ai.DoJavaScript($js) | ConvertFrom-Json)
  $R.outputs = @((File-Info "$D\out.png"), (File-Info "$D\out.pdf"))
  $R.quit = Quit-App 'Illustrator' $ai
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Illustrator' }
Finish $R
