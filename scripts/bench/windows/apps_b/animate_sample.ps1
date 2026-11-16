# Animate on the image's own sample documents (shipped with the application): a JSFL script opens one, reads its size and timeline,
# exports a frame as PNG, closes it and quits.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'animatesample'
try {
  $f = Get-ChildItem 'C:\Program Files\Adobe\Adobe Animate 2027' -Recurse -Filter 'Hippo.fla' -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $f) { throw 'shipped sample document not found' }
  $R.source = $f.Name; $R.source_bytes = $f.Length
  $u = 'file:///' + ($D -replace '\\', '/' -replace ':', '|')
  $src = 'file:///' + ($f.FullName -replace '\\', '/' -replace ':', '|' -replace ' ', '%20')
  $jsfl = @"
var out = '$u', log = [];
try {
  var dom = fl.openDocument('$src');
  log.push('opened');
  var tl = dom.getTimeline();
  log.push('size=' + dom.width + 'x' + dom.height);
  log.push('frames=' + tl.frameCount);
  log.push('layers=' + tl.layerCount);
  dom.exportPNG(out + '/out.png', true, true); log.push('png');
  fl.closeDocument(dom, false);
} catch (e) { log.push('ERR ' + String(e)); }
FLfile.write(out + '/jsfl_log.txt', log.join(','));
fl.quit(false);
"@
  Set-Content "$D\run.jsfl" $jsfl -Encoding UTF8
  $p = Start-Process 'C:\Program Files\Adobe\Adobe Animate 2027\Animate.exe' -ArgumentList "`"$D\run.jsfl`"" -PassThru
  $t0 = Get-Date
  while (-not $p.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt 300) { [void](Dismiss-Dialogs 'Animate'); Start-Sleep 3 }
  if (-not $p.HasExited) { $R.quit = Quit-App 'Animate'; throw 'Animate did not finish the script' } else { $R.quit = 'process' }
  $R.jsfl_log = [string](Get-Content "$D\jsfl_log.txt" -Raw -ErrorAction SilentlyContinue)
  $R.outputs = @(); if (Test-Path "$D\out.png") { $R.outputs += (File-Info "$D\out.png") }
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'Animate' } }
Finish $R
