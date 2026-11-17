# Animate opens the open-licence FLA projects of the data volume, records size and timeline counts, exports a PNG of each.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'animateprojects'
try {
  $DATA = Data-Root
  $files = @(Get-ChildItem (Join-Path $DATA 'project') -Filter *.fla | Sort-Object Name)
  $R.count = $files.Count
  if ($files.Count -eq 0) { throw 'no project files on the data volume' }
  $u = 'file:///' + ($D -replace '\\', '/' -replace ':', '|')
  $lines = for ($i = 0; $i -lt $files.Count; $i++) { "'" + ('file:///' + ($files[$i].FullName -replace '\\', '/' -replace ':', '|' -replace ' ', '%20')) + "'" }
  $jsfl = @"
var out = '$u', files = [$($lines -join ',')], log = [];
for (var i = 0; i < files.length; i++) {
  try {
    var dom = fl.openDocument(files[i]);
    var tl = dom.getTimeline();
    log.push('OK ' + i + ':' + dom.width + 'x' + dom.height + ':F' + tl.frameCount + ':L' + tl.layerCount);
    dom.exportPNG(out + '/f' + i + '.png', true, true);
    fl.closeDocument(dom, false);
  } catch (e) { log.push('ERR ' + i + ':' + String(e).substring(0, 60)); }
}
FLfile.write(out + '/jsfl_log.txt', log.join('||'));
fl.quit(false);
"@
  Set-Content "$D\run.jsfl" $jsfl -Encoding UTF8
  $p = Start-Process 'C:\Program Files\Adobe\Adobe Animate 2027\Animate.exe' -ArgumentList "`"$D\run.jsfl`"" -PassThru
  $t0 = Get-Date
  while (-not $p.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt 400) { [void](Dismiss-Dialogs 'Animate'); Start-Sleep 3 }
  if (-not $p.HasExited) { $R.quit = Quit-App 'Animate'; throw 'Animate did not finish the script' } else { $R.quit = 'process' }
  $R.rows = [string](Get-Content "$D\jsfl_log.txt" -Raw -ErrorAction SilentlyContinue)
  $R.outputs = @(Get-ChildItem $D -Filter f*.png | ForEach-Object { File-Info $_.FullName })
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'Animate' } }
Finish $R
