
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

$D='C:\kfapps\498e3c9611c24278a3fa7613e190a785\solidworks'
New-Item -ItemType Directory -Path $D -ErrorAction Stop | Out-Null
$r=@{app='solidworks';run_id='498e3c9611c24278a3fa7613e190a785';started=(Epoch (Get-Date));kind='PROBE'}
try {

# Measured 2026-11-14: PowerShell's COM adapter fails every SldWorks call with TYPE_E_ELEMENTNOTFOUND, but
# late-bound IDispatch InvokeMember works (RevisionNumber 34.5.2, OpenDoc6, SaveAs). Optional IDispatch
# arguments must be a DispatchWrapper around null; ints by reference need a ParameterModifier.
$ComT=[System.__ComObject]
$bf=[Reflection.BindingFlags]
function Com-Call($obj,$name,$kind,$argv,$byref){
 $pm=$null
 if($byref -and $byref.Count -gt 0){$m=New-Object Reflection.ParameterModifier $argv.Count;foreach($i in $byref){$m[$i]=$true};$pm=@($m)}
 $ComT.InvokeMember($name,$bf::$kind,$null,$obj,$argv,$pm,$null,$null)
}
$r.step='create'
$t=[Type]::GetTypeFromProgID('SldWorks.Application')
$sw=[Activator]::CreateInstance($t)
$r.step='visible'
[void](Com-Call $sw 'Visible' SetProperty @($true) @())
$r.revision=[string](Com-Call $sw 'RevisionNumber' InvokeMethod @() @())
$part=Get-ChildItem 'C:\ProgramData\SolidWorks' -Recurse -Filter *.sldprt -ErrorAction SilentlyContinue | Sort-Object Length | Select-Object -First 1
if(-not $part){throw 'No sample part under C:\ProgramData\SolidWorks'}
$r.step='open'
$a=[object[]]@($part.FullName,[int]1,[int]1,'',[int]0,[int]0)
$model=Com-Call $sw 'OpenDoc6' InvokeMethod $a @(4,5)
if(-not $model -or $a[4] -ne 0){throw "OpenDoc6 failed err=$($a[4])"}
$r.opened_title=[string](Com-Call $model 'GetTitle' InvokeMethod @() @())
$r.step='saveas'
$ext=Com-Call $model 'Extension' GetProperty @() @()
$b=[object[]]@("$D\model.step",[int]0,[int]1,[System.Runtime.InteropServices.DispatchWrapper]::new([object]$null),[int]0,[int]0)
$ok=Com-Call $ext 'SaveAs' InvokeMethod $b @(4,5)
$r.step='stat'
$f=Get-Item "$D\model.step"
$r.kind='FILE';$r.artifact='model.step';$r.modified=Epoch $f.LastWriteTimeUtc;$r.bytes=$f.Length
$r.api_ok=([bool]$ok -and $b[4] -eq 0)
$r.step='exit'
try{[void](Com-Call $sw 'ExitApp' InvokeMethod @() @())}catch{}
Get-Process SLDWORKS -ErrorAction SilentlyContinue | ForEach-Object {Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue}
} catch { $r.error=$_.Exception.Message } finally {
try {Save-Screen "$D\screen.png"} catch {$r.screen_error=$_.Exception.Message}
$r.ended=Epoch (Get-Date)
ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\result.tmp"
Move-Item "$D\result.tmp" "$D\result.json"
}
