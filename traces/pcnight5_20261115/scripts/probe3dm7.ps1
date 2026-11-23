Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfM { [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
'@
[KfM]::SetProcessDPIAware() | Out-Null
function Win(){ foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){ $c=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id); foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$c)){ if($w.Current.Name -eq '3DMark'){return $w} } } }
function Els(){ @((Win).FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)) }
function Inv($name){ $e=Els|?{$_.Current.Name -eq $name -and $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Hyperlink'}|select -First 1; if($e){$e.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); "inv $name"}else{"missing $name"} }
$o=@()
$o+=Inv 'HOME'; Start-Sleep 3; $o+=Inv 'BENCHMARKS'; Start-Sleep 4; $o+=Inv 'DIRECTX 12'; Start-Sleep 3
$wd=[BitConverter]::ToUInt32([BitConverter]::GetBytes([int]-240),0)
for($i=0;$i -lt 30;$i++){
 $card=Els|?{$_.Current.Name -eq 'Time Spy Extreme' -and $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text'}|select -First 1
 $r=$card.Current.BoundingRectangle; $o+="iter $i rect $r"
 if(-not $r.IsEmpty -and $r.Top -gt 450 -and $r.Bottom -lt 950){ $x=[int]($r.Left-640+150);$y=[int]($r.Top-150); [KfM]::SetCursorPos($x,$y)|Out-Null; [KfM]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [KfM]::mouse_event(4,0,0,0,[UIntPtr]::Zero); $o+="clicked $x,$y"; Start-Sleep 4; break }
 [KfM]::SetCursorPos(900,600)|Out-Null; [KfM]::mouse_event(0x800,0,0,$wd,[UIntPtr]::Zero); Start-Sleep 2
}
$o+=(Els|?{$_.Current.ControlType.ProgrammaticName -match 'Text|Hyperlink' -and $_.Current.Name -match 'Time Spy|^RUN|Enable Sound'}|%{"$($_.Current.ControlType.ProgrammaticName)|$($_.Current.Name)@$([int]$_.Current.BoundingRectangle.Left),$([int]$_.Current.BoundingRectangle.Top)"})
$o | Set-Content -Encoding UTF8 C:\kfhw\probe7.txt
