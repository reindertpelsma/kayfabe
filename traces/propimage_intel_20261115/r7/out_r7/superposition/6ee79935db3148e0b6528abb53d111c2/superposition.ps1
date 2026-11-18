
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

$D='C:\kfapps\6ee79935db3148e0b6528abb53d111c2\superposition'
New-Item -ItemType Directory -Path $D -ErrorAction Stop | Out-Null
$r=@{app='superposition';run_id='6ee79935db3148e0b6528abb53d111c2';started=(Epoch (Get-Date));kind='PROBE'}
try {
$source=Join-Path $env:USERPROFILE 'Superposition\log.html'; Remove-Item $source -ErrorAction SilentlyContinue
$p=Start-Process 'C:\Program Files\Unigine\Superposition Benchmark\bin\launcher.exe' -PassThru; Start-Sleep 30
$p.Refresh();$r.kind='LAUNCH_ONLY';$r.launched=(-not $p.HasExited);$r.title=$p.MainWindowTitle
try {Dump-Uia $p.Id} catch {$r.uia_error=$_.Exception.Message}

Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfClick {
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
'@
[KfClick]::SetProcessDPIAware() | Out-Null
$cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$p.Id)
$wins=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)
if($wins.Count -ne 1){throw 'Launcher window missing or ambiguous'}
$rect=$wins[0].Current.BoundingRectangle
if($rect.IsEmpty -or $rect.Width -le 0 -or $rect.Height -le 0){throw 'No UIA window rectangle'}
if(-not [KfClick]::SetForegroundWindow($p.MainWindowHandle)){throw 'Cannot focus launcher'}
Save-Screen "$D\before.png"
$x=[int]($rect.Left+$rect.Width*(927.0/1024.0))
$y=[int]($rect.Top+$rect.Height*(602.0/640.0))
[KfClick]::SetCursorPos($x,$y) | Out-Null
[KfClick]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
[KfClick]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
$deadline=(Get-Date).AddMinutes(10)
while((Get-Date) -lt $deadline){
 if(Test-Path $source){$text=Get-Content $source -Raw;if($text -match 'Benchmark running' -and $text -match 'Benchmark stopped'){break}}
 Start-Sleep 5
}
Save-Screen "$D\result.png"
$f=Get-Item $source
Copy-Item $source "$D\log.html"
$r.kind='FILE';$r.artifact='log.html';$r.modified=Epoch $f.LastWriteTimeUtc;$r.bytes=$f.Length
} catch { $r.error=$_.Exception.Message } finally {
try {Save-Screen "$D\screen.png"} catch {$r.screen_error=$_.Exception.Message}
$r.ended=Epoch (Get-Date)
ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\result.tmp"
Move-Item "$D\result.tmp" "$D\result.json"
}
