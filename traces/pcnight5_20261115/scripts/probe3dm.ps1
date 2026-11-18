Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$o=@()
foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){
 $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id)
 foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)){ if($w.Current.Name -eq '3DMark'){$win=$w} }
}
if(-not $win){ "nowin" | Set-Content C:\kfhw\probe.txt; return }
function Dump($tag){ $o=@("=== $tag"); foreach($e in $win.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)){ $c=$e.Current; if($c.Name){ $b=$c.BoundingRectangle; $o+="$($c.ControlType.ProgrammaticName)|$($c.Name)|$($c.AutomationId)|$([int]$b.Left),$([int]$b.Top)|$(($e.GetSupportedPatterns()|%{$_.ProgrammaticName}) -join ',')" } }; $o }
$all=Dump 'home'
$tab=$win.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition) | ?{$_.Current.Name -eq 'BENCHMARKS'} | select -First 1
if($tab){ try{ $tab.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); $all+="invoked" }catch{ $all+="invoke failed: $($_.Exception.Message)" }; Start-Sleep 6; $all+=Dump 'benchmarks' }
$all | Set-Content -Encoding UTF8 C:\kfhw\probe.txt
