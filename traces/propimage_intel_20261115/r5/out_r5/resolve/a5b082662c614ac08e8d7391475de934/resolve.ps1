
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

$D='C:\kfapps\a5b082662c614ac08e8d7391475de934\resolve'
New-Item -ItemType Directory -Path $D -ErrorAction Stop | Out-Null
$r=@{app='resolve';run_id='a5b082662c614ac08e8d7391475de934';started=(Epoch (Get-Date));kind='PROBE'}
try {

Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfFocus {
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern void keybd_event(byte vk,byte sc,uint f,UIntPtr e);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a,uint b,bool f);
 [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
}
'@
function Focus-Win($w){
 $h=[IntPtr]$w.Current.NativeWindowHandle;$fg=[KfFocus]::GetForegroundWindow();$d=0
 $ft=[KfFocus]::GetWindowThreadProcessId($fg,[ref]$d);$me=[KfFocus]::GetCurrentThreadId()
 [KfFocus]::AttachThreadInput($me,$ft,$true) | Out-Null
 [KfFocus]::keybd_event(0x12,0,0,[UIntPtr]::Zero);[KfFocus]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
 [KfFocus]::SetForegroundWindow($h) | Out-Null
 [KfFocus]::AttachThreadInput($me,$ft,$false) | Out-Null
 ([KfFocus]::GetForegroundWindow() -eq $h)
}
function Resolve-Win(){
 foreach($pr in Get-Process -Name Resolve -ErrorAction SilentlyContinue){
  $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id)
  foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)){
   if($w.Current.AutomationId -in 'UiProjectManagerDialog','UiMainWindow'){return $w}
  }
 }
}
$name='kf'+$r.run_id.Substring(0,8)
$r.project_name=$name;$r.window_name='';$r.stage='';$r.log_lines=@()
$t0=Get-Date
$p=Start-Process 'C:\Program Files\Blackmagic Design\DaVinci Resolve\Resolve.exe' -PassThru
$deadline=(Get-Date).AddSeconds(300);$w=$null
while((Get-Date) -lt $deadline){$w=Resolve-Win;if($w -and $w.Current.AutomationId -eq 'UiProjectManagerDialog'){break};Start-Sleep 3}
if(-not $w -or $w.Current.AutomationId -ne 'UiProjectManagerDialog'){$r.stage='no project manager';throw 'Resolve Project Manager window never appeared in UI Automation'}
$r.stage='project manager'
$new=$w.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition) | Where-Object {$_.Current.Name -ceq 'New Project' -and $_.Current.AutomationId.EndsWith('btnAddProject')} | Select-Object -First 1
if(-not $new){throw 'New Project button not found by AutomationId'}
$new.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep 4
$r.focus=Focus-Win $w
[System.Windows.Forms.SendKeys]::SendWait('^a');Start-Sleep -Milliseconds 300
[System.Windows.Forms.SendKeys]::SendWait($name);Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
$r.stage='create sent'
$deadline=(Get-Date).AddSeconds(120)
while((Get-Date) -lt $deadline){$m=Resolve-Win;if($m -and $m.Current.AutomationId -eq 'UiMainWindow'){$r.window_name=$m.Current.Name;if($r.window_name -like "*$name*"){break}};Start-Sleep 3}
$r.stage='main window'
Start-Sleep 5
$log="$env:APPDATA\Blackmagic Design\DaVinci Resolve\Support\logs\davinci_resolve.log"
$lines=@()
foreach($ln in Get-Content $log -ErrorAction SilentlyContinue){
 $m=[regex]::Match($ln,'\| (\d{4}-\d\d-\d\d \d\d:\d\d:\d\d),\d+ \| (.*)$')
 if($m.Success){$ts=[datetime]::ParseExact($m.Groups[1].Value,'yyyy-MM-dd HH:mm:ss',$null);if($ts -ge $t0.AddSeconds(-2) -and $m.Groups[2].Value -match 'Selected compute API|initialised as decoding|Initialized OpenGL|Let There Be CUDA|Detected 1 GPUs'){$lines+=$ln.Substring(0,[Math]::Min(260,$ln.Length))}}
}
$r.log_lines=@($lines | Select-Object -First 40)
$r.kind='RESOLVE'
try {Save-Screen "$D\resolve.png"} catch {}
Get-Process -Name Resolve -ErrorAction SilentlyContinue | ForEach-Object {Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue}
} catch { $r.error=$_.Exception.Message } finally {
try {Save-Screen "$D\screen.png"} catch {$r.screen_error=$_.Exception.Message}
$r.ended=Epoch (Get-Date)
ConvertTo-Json -InputObject $r -Depth 6 | Set-Content -Encoding UTF8 "$D\result.tmp"
Move-Item "$D\result.tmp" "$D\result.json"
}
