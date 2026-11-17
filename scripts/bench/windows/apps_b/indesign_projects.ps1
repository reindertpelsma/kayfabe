# InDesign opens the open-licence INDD projects of the data volume and exports each as PDF; one failing file does not stop the others.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'indesignprojects'
try {
  $DATA = Data-Root
  $files = @(Get-ChildItem (Join-Path $DATA 'project') -Filter *.indd | Sort-Object Name)
  $R.count = $files.Count
  if ($files.Count -eq 0) { throw 'no project files on the data volume' }
  Start-Process 'C:\Program Files\Adobe\Adobe InDesign 2027\InDesign.exe'
  [void](Wait-For { Has-Window 'InDesign' } 'InDesign' 150)
  [void](Wait-For { $false } 'InDesign' 15)
  $id = $null
  for ($i = 0; $i -lt 20 -and -not $id; $i++) { try { $id = New-Object -ComObject 'InDesign.Application.2027' } catch { [void](Dismiss-Dialogs 'InDesign'); Start-Sleep 3 } }
  if (-not $id) { throw 'InDesign COM not available' }
  $list = ($files | ForEach-Object { $_.FullName -replace '\\', '/' }) -join '|'
  $js = @"
app.scriptPreferences.userInteractionLevel = UserInteractionLevels.NEVER_INTERACT;
app.linkingPreferences.checkLinksAtOpen = false;
var files = '$list'.split('|'), rows = [];
for (var i = 0; i < files.length; i++) {
  var name = files[i].split('/').pop();
  try {
    var d = app.open(File(files[i]), false);
    var r = name + ':P' + d.pages.length;
    d.exportFile(ExportFormat.PDF_TYPE, File('$($D -replace '\\','/')/q' + i + '.pdf'), false);
    d.close(SaveOptions.NO); rows.push('OK ' + r);
  } catch (e) { rows.push('ERR ' + name + ':' + String(e).substring(0, 60)); }
}
rows.join('||');
"@
  $R.rows = [string]($id.DoScript($js, 1246973031))
  $R.outputs = @(Get-ChildItem $D -Filter q*.pdf | ForEach-Object { File-Info $_.FullName })
  $R.quit = Quit-App 'InDesign' $id
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'InDesign' }
Finish $R
