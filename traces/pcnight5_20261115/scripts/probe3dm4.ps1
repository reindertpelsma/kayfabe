Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
function Win(){ foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){ $c=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id); foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$c)){ if($w.Current.Name -eq '3DMark'){return $w} } } }
$w=Win; $o=@()
foreach($e in $w.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)){ $c=$e.Current; if($c.Name -and $c.ControlType.ProgrammaticName -eq 'ControlType.Text'){ $o+="$($c.Name)@$([int]$c.BoundingRectangle.Left),$([int]$c.BoundingRectangle.Top)" } }
$o | Set-Content -Encoding UTF8 C:\kfhw\probe4.txt
