# InDesign crash isolation: the row name ends in the step (indesign_s1 .. indesign_s6); each row runs the steps up to its own number and
# writes id_progress.txt after every step, so the file names the step during which InDesign crashed (a missing next line + the crash events).
#  s1 start + COM    s2 new 2-page document    s3 text frames with 60 paragraphs    s4 place the photo    s5 export PDF    s6 export JPEG
. 'C:\kfhw\lib.ps1'
$R = Init-Run $APPNAME
$N = [int]($APPNAME -replace '^indesign_s', '')
function IdP($m) { Add-Content "$D\id_progress.txt" ("{0} {1}" -f [int](Epoch), $m) }
try {
  $DATA = Data-Root
  $photo = (Find-Asset 'lady_fern_4845x2980.jpg').FullName
  $R.source = Split-Path $photo -Leaf
  $R.steps_wanted = $N
  IdP 'launch'
  Start-Process 'C:\Program Files\Adobe\Adobe InDesign 2027\InDesign.exe'
  [void](Wait-For { Has-Window 'InDesign' } 'InDesign' 150)
  [void](Wait-For { $false } 'InDesign' 15)
  $id = $null
  for ($i = 0; $i -lt 20 -and -not $id; $i++) { try { $id = New-Object -ComObject 'InDesign.Application.2027' } catch { [void](Dismiss-Dialogs 'InDesign'); Start-Sleep 3 } }
  if (-not $id) { throw 'InDesign COM not available' }
  $R.version = $id.Version
  IdP 's1 com ok'
  $steps = @(
    'app.scriptPreferences.userInteractionLevel = UserInteractionLevels.NEVER_INTERACT; var doc = app.documents.add(); doc.documentPreferences.pagesPerDocument = 2; doc.documentPreferences.pageWidth = "210mm"; doc.documentPreferences.pageHeight = "297mm"; "doc"',
    'var doc = app.documents[0]; var s = ""; for (var i = 0; i < 60; i++) s += "kayfabe layout test paragraph " + i + " lorem ipsum dolor sit amet. "; var tf = doc.pages[0].textFrames.add({geometricBounds: [20, 20, 120, 190]}); tf.contents = s; var tf2 = doc.pages[1].textFrames.add({geometricBounds: [20, 20, 200, 190]}); tf2.contents = s + s; "text"',
    ('var doc = app.documents[0]; var img = doc.pages[0].rectangles.add({geometricBounds: [130, 20, 270, 190]}); img.place(File("' + ($photo -replace '\\', '/') + '")); img.fit(FitOptions.FILL_PROPORTIONALLY); "place"'),
    ('var doc = app.documents[0]; doc.exportFile(ExportFormat.PDF_TYPE, File("' + ($D -replace '\\', '/') + '/out.pdf"), false); "pdf"'),
    ('var doc = app.documents[0]; app.jpegExportPreferences.pageString = "1"; app.jpegExportPreferences.exportResolution = 150; app.jpegExportPreferences.jpegQuality = JPEGOptionsQuality.HIGH; doc.exportFile(ExportFormat.JPG, File("' + ($D -replace '\\', '/') + '/out.jpg"), false); "jpg"')
  )
  for ($k = 0; $k -lt [Math]::Min($N - 1, $steps.Count); $k++) {
    $dres = $id.DoScript($steps[$k], 1246973031)
    IdP ('s{0} done: {1}' -f ($k + 2), $dres)
  }
  $R.outputs = @(); foreach ($f in 'out.pdf', 'out.jpg') { if (Test-Path "$D\$f") { $R.outputs += (File-Info "$D\$f") } }
  try { $id.DoScript('app.documents.everyItem().close(SaveOptions.NO); "closed"', 1246973031) | Out-Null } catch {}
  $R.quit = Quit-App 'InDesign' $id
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'InDesign' }
Finish $R
