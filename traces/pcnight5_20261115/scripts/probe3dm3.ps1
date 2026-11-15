Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfM { [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
'@
[KfM]::SetProcessDPIAware() | Out-Null
function Win(){ foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){ $c=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id); foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$c)){ if($w.Current.Name -eq '3DMark'){return $w} } } }
function Els($w){ @($w.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)) }
$o=@()
Add-Type -AssemblyName System.Windows.Forms
[KfM]::SetCursorPos(1000,560)|Out-Null; [KfM]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [KfM]::mouse_event(4,0,0,0,[UIntPtr]::Zero); Start-Sleep 1
for($i=0;$i -lt 14;$i++){
 $w=Win; $els=Els $w
 $names=($els|?{$_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $_.Current.Name -match '^(Time Spy|Fire Strike|Port Royal|Night Raid|Wild Life|Steel|Speed|Solar|DirectX|API|Mesh|PCI|Cloud)'}|%{ "$($_.Current.Name)@$([int]$_.Current.BoundingRectangle.Left),$([int]$_.Current.BoundingRectangle.Top)" }) -join ' ; '
 $o+="scroll $i : $names"
 [System.Windows.Forms.SendKeys]::SendWait('{PGDN}')
 Start-Sleep 2
}

$o | Set-Content -Encoding UTF8 C:\kfhw\probe3.txt
