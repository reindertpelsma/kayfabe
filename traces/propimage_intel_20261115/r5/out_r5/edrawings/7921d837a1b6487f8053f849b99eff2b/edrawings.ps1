
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

$D='C:\kfapps\7921d837a1b6487f8053f849b99eff2b\edrawings'
New-Item -ItemType Directory -Path $D -ErrorAction Stop | Out-Null
$r=@{app='edrawings';run_id='7921d837a1b6487f8053f849b99eff2b';started=(Epoch (Get-Date));kind='PROBE'}
try {

Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfMsg { [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,IntPtr l); }
'@
function Find-Ui($root,$test){
 $q=New-Object System.Collections.Queue;$q.Enqueue($root);$n=0
 while($q.Count -gt 0 -and $n -lt 800){$el=$q.Dequeue();$n++
  try{foreach($ch in $el.FindAll([System.Windows.Automation.TreeScope]::Children,[System.Windows.Automation.Condition]::TrueCondition)){ if(& $test $ch){return $ch}; $q.Enqueue($ch) }}catch{}
 }
}
$sample=Get-ChildItem 'C:\ProgramData\SolidWorks' -Recurse -Filter *.sldprt -ErrorAction SilentlyContinue | Sort-Object Length | Select-Object -First 1
if(-not $sample){throw 'No sample part under C:\ProgramData\SolidWorks'}
Copy-Item $sample.FullName "$D\kfsample.sldprt"
$r.sample='kfsample.sldprt';$r.accepted=$false;$r.accept_method='';$r.tab_name='';$r.windows_seen=@()
$p=Start-Process 'C:\Program Files\SOLIDWORKS Corp\eDrawings\eDrawings.exe' -ArgumentList "`"$D\kfsample.sldprt`"" -PassThru
$deadline=(Get-Date).AddSeconds(240)
while((Get-Date) -lt $deadline){
 foreach($pr in Get-Process -Name eDrawings -ErrorAction SilentlyContinue){
  $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id)
  foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)){
   $r.windows_seen=@(@($r.windows_seen)+@("$($w.Current.ClassName):$($w.Current.Name)") | Select-Object -Unique -First 12)
   if(-not $r.accepted){
    $btn=Find-Ui $w {param($e) $e.Current.Name -ceq 'ACCEPT' -and $e.Current.AutomationId -ceq '1'}
    if($btn){$h=[IntPtr]$btn.Current.NativeWindowHandle;if($h -ne [IntPtr]::Zero){[KfMsg]::SendMessage($h,0xF5,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null;$r.accepted=$true;$r.accept_method='BM_CLICK on the UIA window handle';Start-Sleep 5}}
   }
   $tab=Find-Ui $w {param($e) $e.Current.ControlType.ProgrammaticName -ceq 'ControlType.TabItem' -and $e.Current.Name -ieq 'kfsample.sldprt'}
   if($tab){$r.tab_name=$tab.Current.Name}
  }
 }
 if($r.tab_name){break}
 Start-Sleep 3
}
$r.kind='EDW'
Start-Sleep 3
try {Save-Screen "$D\edrawings.png"} catch {}
Get-Process -Name eDrawings -ErrorAction SilentlyContinue | ForEach-Object {Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue}
} catch { $r.error=$_.Exception.Message } finally {
try {Save-Screen "$D\screen.png"} catch {$r.screen_error=$_.Exception.Message}
$r.ended=Epoch (Get-Date)
ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\result.tmp"
Move-Item "$D\result.tmp" "$D\result.json"
}
