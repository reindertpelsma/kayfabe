# InDesign on the image's own sample documents (shipped with the application): open one, export a PDF and a page JPEG, quit.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'indesign_sample'
try {
  $f = Get-ChildItem 'C:\Program Files\Adobe\Adobe InDesign 2027' -Recurse -Filter 'SampleFlexLayoutFile.indd' -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $f) { throw 'shipped sample document not found' }
  $R.source = $f.Name; $R.source_bytes = $f.Length
  Start-Process 'C:\Program Files\Adobe\Adobe InDesign 2027\InDesign.exe'
  [void](Wait-For { Has-Window 'InDesign' } 'InDesign' 150)
  [void](Wait-For { $false } 'InDesign' 15)
  $id = $null
  for ($i = 0; $i -lt 20 -and -not $id; $i++) { try { $id = New-Object -ComObject 'InDesign.Application.2027' } catch { [void](Dismiss-Dialogs 'InDesign'); Start-Sleep 3 } }
  if (-not $id) { throw 'InDesign COM not available' }
  $js = @"
app.scriptPreferences.userInteractionLevel = UserInteractionLevels.NEVER_INTERACT;
app.linkingPreferences.checkLinksAtOpen = false;
var doc = app.open(File('$($f.FullName -replace '\\','/')'), false);
var n = doc.pages.length, fr = doc.pages[0].allPageItems.length;
doc.exportFile(ExportFormat.PDF_TYPE, File('$($D -replace '\\','/')/out.pdf'), false);
app.jpegExportPreferences.pageString = '1'; app.jpegExportPreferences.exportResolution = 100; app.jpegExportPreferences.jpegQuality = JPEGOptionsQuality.HIGH;
doc.exportFile(ExportFormat.JPG, File('$($D -replace '\\','/')/out.jpg'), false);
doc.close(SaveOptions.NO);
'{"pages":"' + n + '","items":"' + fr + '"}';
"@
  $R.js = ($id.DoScript($js, 1246973031) | ConvertFrom-Json)
  $R.outputs = @((File-Info "$D\out.pdf"), (File-Info "$D\out.jpg"))
  $R.quit = Quit-App 'InDesign' $id
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'InDesign' }
Finish $R
