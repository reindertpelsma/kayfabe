
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
function Save-Screen($path){
 $b=[System.Windows.Forms.SystemInformation]::VirtualScreen
 $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height
 $g=[System.Drawing.Graphics]::FromImage($bmp)
 try {$g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size);$bmp.Save($path,[System.Drawing.Imaging.ImageFormat]::Png)} finally {$g.Dispose();$bmp.Dispose()}
}
function Epoch($d) { ([DateTimeOffset]$d.ToUniversalTime()).ToUnixTimeMilliseconds()/1000.0 }
function Dump-Uia($pidValue) {
 $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pidValue)
 $roots=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)
 $rows=New-Object System.Collections.Generic.List[object]
 $queue=New-Object System.Collections.Queue
 foreach($root in $roots){$queue.Enqueue($root)}
 while($queue.Count -gt 0 -and $rows.Count -lt 2048){
  $el=$queue.Dequeue()
  try {
   $c=$el.Current
   $rows.Add(@{Name=$c.Name;AutomationId=$c.AutomationId;ControlType=$c.ControlType.ProgrammaticName;ProcessId=$c.ProcessId;Patterns=@($el.GetSupportedPatterns() | ForEach-Object {$_.ProgrammaticName})})
   foreach($child in $el.FindAll([System.Windows.Automation.TreeScope]::Children,[System.Windows.Automation.Condition]::TrueCondition)){
    if($queue.Count -lt 2048){$queue.Enqueue($child)}
   }
  }catch{}
 }
 ConvertTo-Json -InputObject @($rows.ToArray()) -Depth 6 | Set-Content -Encoding UTF8 "$D\uia.json"
}
function Find-Control($selector,$pidValue) {
 $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pidValue)
 $roots=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)
 $found=@(foreach($root in $roots){
  $all=$root.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)
  foreach($el in $all){$c=$el.Current
   if($c.AutomationId -ceq $selector.id -and (-not $selector.name -or $c.Name -ceq $selector.name) -and $c.ControlType.ProgrammaticName -ceq $selector.type){$el}
  }
 })
 if($found.Count -ne 1){throw 'UIA selector missing or ambiguous'}
 return $found[0]
}

$D='C:\kfapps\fbee294b88c34949ae52a814ab83c120\edge'
New-Item -ItemType Directory -Path $D -ErrorAction Stop | Out-Null
$r=@{app='edge';run_id='fbee294b88c34949ae52a814ab83c120';started=(Epoch (Get-Date));kind='PROBE'}
try {

$exe='C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
$page="$D\kfgl.html"
$html=@'
<!doctype html><meta charset=utf-8><title>KFGL pending</title><canvas id=c width=64 height=64></canvas>
<script>
const nonce='__NONCE__';
try {
 const c=document.getElementById('c');
 const gl=c.getContext('webgl2')||c.getContext('webgl');
 const ext=gl.getExtension('WEBGL_debug_renderer_info');
 const r=ext?gl.getParameter(ext.UNMASKED_RENDERER_WEBGL):gl.getParameter(gl.RENDERER);
 let n=0;const t0=performance.now();
 function f(){gl.clearColor((n%255)/255,0.5,0.2,1);gl.clear(gl.COLOR_BUFFER_BIT);n++;
  if(performance.now()-t0<4000){requestAnimationFrame(f);}
  else{document.title='KFGL '+nonce+' ok renderer='+r+' frames='+n;}}
 requestAnimationFrame(f);
} catch(e){document.title='KFGL '+nonce+' err '+e;}
</script>
'@
Set-Content -Encoding UTF8 $page $html.Replace('__NONCE__',$r.run_id)
$profile="$D\edge-profile"
$uri=([System.Uri]$page).AbsoluteUri
$p=Start-Process $exe -ArgumentList "--user-data-dir=`"$profile`"","--no-first-run","--no-default-browser-check","--disable-sync","--new-window",$uri -PassThru
$deadline=(Get-Date).AddSeconds(120)
$r.launched=$false;$r.window_title=''
while((Get-Date) -lt $deadline){
 $procs=@(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" | Where-Object {$_.CommandLine -like "*$profile*"} | ForEach-Object {$_.ProcessId})
 $r.edge_pids=$procs
 foreach($pidv in $procs){
  $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pidv)
  $wins=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)
  foreach($w in $wins){
   $name=$w.Current.Name
   if($name -like 'KFGL *'){$r.launched=$true;$r.window_title=$name}
  }
 }
 if($r.window_title -match ' (ok|err) '){break}
 Start-Sleep -Seconds 2
}
$r.kind='UIA'
try {Save-Screen "$D\edge.png"} catch {}
Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" | Where-Object {$_.CommandLine -like "*$profile*"} | ForEach-Object {Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue}
} catch { $r.error=$_.Exception.Message } finally {
try {Save-Screen "$D\screen.png"} catch {$r.screen_error=$_.Exception.Message}
$r.ended=Epoch (Get-Date)
ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\result.tmp"
Move-Item "$D\result.tmp" "$D\result.json"
}
