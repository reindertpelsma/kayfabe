Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfM { [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
'@
[KfM]::SetProcessDPIAware() | Out-Null
function Win(){ foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){ $c=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id); foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$c)){ if($w.Current.Name -eq '3DMark'){return $w} } } }
function Els(){ @((Win).FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)) }
$o=@()
Add-Type -AssemblyName System.Windows.Forms
[KfM]::SetCursorPos(330,576)|Out-Null; [KfM]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [KfM]::mouse_event(4,0,0,0,[UIntPtr]::Zero); Start-Sleep 1
for($i=0;$i -lt 12;$i++){
 $card=Els|?{$_.Current.Name -eq 'Time Spy INSTALLED' -and $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Custom'}|select -First 1
 $r=$card.Current.BoundingRectangle; $o+="iter $i rect $r"
 if(-not $r.IsEmpty -and $r.Top -gt 100 -and $r.Bottom -lt 1050){ $x=[int]($r.Left+$r.Width/2);$y=[int]($r.Top+$r.Height/2); [KfM]::SetCursorPos($x,$y)|Out-Null; [KfM]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [KfM]::mouse_event(4,0,0,0,[UIntPtr]::Zero); $o+="clicked $x,$y"; Start-Sleep 4; break }
 [System.Windows.Forms.SendKeys]::SendWait('{PGDN}'); Start-Sleep 2
}
$o+=(Els|?{$_.Current.ControlType.ProgrammaticName -match 'Text|Hyperlink' -and $_.Current.Name -match 'Time Spy|^RUN|Custom'}|%{"$($_.Current.ControlType.ProgrammaticName)|$($_.Current.Name)@$([int]$_.Current.BoundingRectangle.Left),$([int]$_.Current.BoundingRectangle.Top)"})
$o | Set-Content -Encoding UTF8 C:\kfhw\probe6.txt
