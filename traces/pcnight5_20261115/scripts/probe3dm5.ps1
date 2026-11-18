Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
function Win(){ foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){ $c=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id); foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$c)){ if($w.Current.Name -eq '3DMark'){return $w} } } }
function Els(){ @((Win).FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)) }
function Inv($name){ $e=Els|?{$_.Current.Name -eq $name -and $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Hyperlink'}|select -First 1; if($e){$e.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); "inv $name"}else{"missing $name"} }
$o=@()
$o+=Inv 'BENCHMARKS'; Start-Sleep 4
$o+=Inv 'DIRECTX 12'; Start-Sleep 4
$o+=(Els|?{$_.Current.ControlType.ProgrammaticName -eq 'ControlType.Custom' -and $_.Current.Name}|%{"CUSTOM $($_.Current.Name)"})
$o+="--- texts"
$o+=(Els|?{$_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $_.Current.Name}|%{"T $($_.Current.Name)@$([int]$_.Current.BoundingRectangle.Left),$([int]$_.Current.BoundingRectangle.Top)"})
$o | Set-Content -Encoding UTF8 C:\kfhw\probe5.txt
