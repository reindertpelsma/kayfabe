# InDesign: build a two-page document from nothing (text frames, a placed photo), export PDF and a page JPEG, quit.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'indesign'
try {
  $DATA = Data-Root
  $photo = (Find-Asset 'lady_fern_4845x2980.jpg').FullName
  $R.source = Split-Path $photo -Leaf
  Step 'launch'
  Start-Process 'C:\Program Files\Adobe\Adobe InDesign 2027\InDesign.exe'
  [void](Wait-For { Has-Window 'InDesign' } 'InDesign' 150)
  [void](Wait-For { $false } 'InDesign' 15)
  Step 'com'
  $id = $null
  for ($i = 0; $i -lt 20 -and -not $id; $i++) { try { $id = New-Object -ComObject 'InDesign.Application.2027' } catch { [void](Dismiss-Dialogs 'InDesign'); Start-Sleep 3 } }
  if (-not $id) { throw 'InDesign COM not available' }
  $R.version = $id.Version
  $js = @"
app.scriptPreferences.userInteractionLevel = UserInteractionLevels.NEVER_INTERACT;
var doc = app.documents.add();
doc.documentPreferences.pagesPerDocument = 2; doc.documentPreferences.pageWidth = '210mm'; doc.documentPreferences.pageHeight = '297mm';
var p1 = doc.pages[0], p2 = doc.pages[1];
var tf = p1.textFrames.add({geometricBounds: [20, 20, 120, 190]});
var s = ''; for (var i = 0; i < 60; i++) s += 'kayfabe layout test paragraph ' + i + ' lorem ipsum dolor sit amet. ';
tf.contents = s;
var img = p1.rectangles.add({geometricBounds: [130, 20, 270, 190]});
img.place(File('$($photo -replace '\\','/')'));
img.fit(FitOptions.FILL_PROPORTIONALLY);
var tf2 = p2.textFrames.add({geometricBounds: [20, 20, 200, 190]});
tf2.contents = s + s;
var pdfOk = false;
doc.exportFile(ExportFormat.PDF_TYPE, File('$($D -replace '\\','/')/out.pdf'), false);
app.jpegExportPreferences.pageString = '1'; app.jpegExportPreferences.exportResolution = 150; app.jpegExportPreferences.jpegQuality = JPEGOptionsQuality.HIGH;
doc.exportFile(ExportFormat.JPG, File('$($D -replace '\\','/')/out.jpg'), false);
var n = doc.pages.length; var g = img.allGraphics.length;
doc.close(SaveOptions.NO);
'{"pages":"' + n + '","graphics":"' + g + '"}';
"@
  $res = $id.DoScript($js, 1246973031)
  $R.js = ($res | ConvertFrom-Json)
  $R.outputs = @((File-Info "$D\out.pdf"), (File-Info "$D\out.jpg"))
  $R.quit = Quit-App 'InDesign' $id
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'InDesign' }
Finish $R
