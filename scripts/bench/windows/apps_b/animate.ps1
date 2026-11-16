# Animate: run a JSFL script that builds a document (shape, motion tween), saves it and exports a PNG of a frame, then quits.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'animate'
try {
  $u = 'file:///' + ($D -replace '\\', '/' -replace ':', '|')
  $jsfl = @"
var out = '$u';
var log = [];
try {
  var dom = fl.createDocument('timeline');
  dom.width = 1280; dom.height = 720;
  dom.addNewRectangle({left: 100, top: 100, right: 400, bottom: 300}, 0);
  dom.selectAll(); dom.convertToSymbol('movie clip', 'kfbox', 'center');
  dom.getTimeline().insertFrames(30, true, 0);
  dom.getTimeline().createMotionTween(0, 0);
  dom.getTimeline().setSelectedFrames(29, 29);
  dom.getTimeline().insertKeyframe(29);
  log.push('doc');
  fl.saveDocument(dom, out + '/out.fla'); log.push('saved');
  dom.exportPNG(out + '/out.png', true, true); log.push('png');
  FLfile.write(out + '/jsfl_log.txt', log.join(','));
  fl.closeDocument(dom, false);
} catch (e) { FLfile.write(out + '/jsfl_log.txt', 'ERR ' + String(e) + ' | ' + log.join(',')); }
fl.quit(false);
"@
  Set-Content "$D\run.jsfl" $jsfl -Encoding UTF8
  $p = Start-Process 'C:\Program Files\Adobe\Adobe Animate 2027\Animate.exe' -ArgumentList "`"$D\run.jsfl`"" -PassThru
  $t0 = Get-Date
  while (-not $p.HasExited -and ((Get-Date) - $t0).TotalSeconds -lt 240) { [void](Dismiss-Dialogs 'Animate'); Start-Sleep 3 }
  if (-not $p.HasExited) { $R.quit = Quit-App 'Animate'; throw 'Animate did not finish the script' } else { $R.quit = 'process' }
  $R.jsfl_log = (Get-Content "$D\jsfl_log.txt" -Raw -ErrorAction SilentlyContinue)
  $R.outputs = @(); foreach ($f in 'out.png', 'out.fla') { if (Test-Path "$D\$f") { $R.outputs += (File-Info "$D\$f") } }
} catch { $R.error = $_.Exception.Message; if (-not $R.quit) { $R.quit = Quit-App 'Animate' } }
Finish $R
