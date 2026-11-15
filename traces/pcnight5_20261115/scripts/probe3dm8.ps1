Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$o=@()
try{
foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){ $c=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id); foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$c)){ if($w.Current.Name -eq '3DMark'){$win=$w} } }
foreach($e in $win.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)){ $c=$e.Current; if($c.Name -match 'Time Spy'){ $p=($e.GetSupportedPatterns()|%{$_.ProgrammaticName}) -join ','; $o+="$($c.ControlType.ProgrammaticName)|$($c.Name)|$($c.BoundingRectangle)|$p|off=$($c.IsOffscreen)" } }
}catch{ $o+="ERR $($_.Exception.Message)" }
$o | Set-Content -Encoding UTF8 C:\kfhw\probe8.txt
