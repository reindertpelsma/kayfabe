
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

$D='C:\kfapps\8c88046bbff74276a6a0fab1dc564fce\3dmark'
New-Item -ItemType Directory -Path $D -ErrorAction Stop | Out-Null
$r=@{app='3dmark';run_id='8c88046bbff74276a6a0fab1dc564fce';started=(Epoch (Get-Date));kind='PROBE'}
try {

$exe='C:\Program Files\UL\3DMark\3DMark.exe'
$cmd='C:\Program Files\UL\3DMark\3DMarkCmd.exe'
$r.cli_output=''
if(Test-Path $cmd){
 $o=(& $cmd --definition=timespy.3dmdef --online=off --audio=off --systeminfo=off --out="$D\cli.3dmark-result" 2>&1 | Out-String)
 $r.cli_output=$o.Substring(0,[Math]::Min(600,$o.Length))
}
$docs="$env:USERPROFILE\Documents\3DMark"
$started=Get-Date
$p=Start-Process $exe -ArgumentList '--force-renderer-accessibility' -PassThru
function Main-Window(){
 foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){
  $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id)
  foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)){ if($w.Current.Name -eq '3DMark'){return $w} }
 }
}
function Ui-Elements($w){ @($w.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)) }
function Ui-Texts($els){ ($els | ForEach-Object {$_.Current.Name} | Where-Object {$_} | Select-Object -Unique -First 60) -join ' ; ' }
$r.ui_ready=$false;$r.progress_seen=$false;$r.result_file='';$r.ui_texts=''
$deadline=(Get-Date).AddSeconds(240);$els=@()
while((Get-Date) -lt $deadline){
 $w=Main-Window
 if($w){$els=Ui-Elements $w; if(@($els | Where-Object {$_.Current.Name -eq 'BENCHMARKS' -or $_.Current.Name -eq 'HOME'}).Count -gt 0 -and @($els | Where-Object {$_.Current.Name -eq 'Time Spy'}).Count -gt 0){$r.ui_ready=$true;break}}
 Start-Sleep 5
}
if(-not $r.ui_ready){$r.ui_texts=Ui-Texts $els;throw 'Time Spy card not visible in the UI Automation tree (UI never became ready)'}
$title=$els | Where-Object {$_.Current.Name -eq 'Time Spy' -and $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text'} | Select-Object -First 1
$tl=$title.Current.BoundingRectangle
$run=$null;$best=99999
foreach($e in $els){$c=$e.Current;if($c.Name -eq 'RUN' -and $c.ControlType.ProgrammaticName -eq 'ControlType.Hyperlink'){$b=$c.BoundingRectangle;if([math]::Abs($b.Left-$tl.Left) -lt 6 -and $b.Top -gt $tl.Top -and ($b.Top-$tl.Top) -lt $best){$best=$b.Top-$tl.Top;$run=$e}}}
if(-not $run){throw 'No RUN link under the Time Spy card'}
$run.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
$r.invoked=Epoch (Get-Date)
$deadline=(Get-Date).AddSeconds(1080)
while((Get-Date) -lt $deadline){
 Start-Sleep 5
 $w=Main-Window
 if($w){$els=Ui-Elements $w;$t=Ui-Texts $els;if($t){$r.ui_texts=$t};if($t -match 'Running benchmark'){$r.progress_seen=$true}}
 $f=Get-ChildItem $docs -Filter *.3dmark-result -ErrorAction SilentlyContinue | Where-Object {$_.LastWriteTime -gt $started} | Sort-Object LastWriteTime -Descending | Select-Object -First 1
 if($f){Start-Sleep 5;$f=Get-Item $f.FullName;Copy-Item $f.FullName "$D\result.3dmark-result";$r.result_file=$f.Name;$r.modified=Epoch $f.LastWriteTimeUtc;$r.bytes=$f.Length;break}
}
$r.kind='TDM'
try {Save-Screen "$D\3dmark.png"} catch {}
Get-Process | Where-Object {$_.Name -match '^3DMark'} | ForEach-Object {Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue}
} catch { $r.error=$_.Exception.Message } finally {
try {Save-Screen "$D\screen.png"} catch {$r.screen_error=$_.Exception.Message}
$r.ended=Epoch (Get-Date)
ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\result.tmp"
Move-Item "$D\result.tmp" "$D\result.json"
}
