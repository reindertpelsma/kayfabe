Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$UI='System.Windows.Automation'
if(-not (Get-Process -Name 3DMark -ErrorAction SilentlyContinue)){ Start-Process 'C:\Program Files\UL\3DMark\3DMark.exe' -ArgumentList '--force-renderer-accessibility' | Out-Null }
function Win(){ foreach($pr in Get-Process -Name 3DMark -ErrorAction SilentlyContinue){ $c=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id); foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$c)){ if($w.Current.Name -eq '3DMark'){return $w} } } }
function Els($w){ @($w.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)) }
function Dump($els,$tag){ $o=@("=== $tag"); foreach($e in $els){ $c=$e.Current; if($c.Name){ $b=$c.BoundingRectangle; $o+="$($c.ControlType.ProgrammaticName)|$($c.Name)|$($c.AutomationId)|$([int]$b.Left),$([int]$b.Top),$([int]$b.Width)x$([int]$b.Height)|$(($e.GetSupportedPatterns()|%{$_.ProgrammaticName.Replace('PatternIdentifiers.Pattern','')}) -join ',')" } }; $o }
$all=@()
$dl=(Get-Date).AddSeconds(120); $els=@()
while((Get-Date) -lt $dl){ $w=Win; if($w){ $els=Els $w; if(@($els|?{$_.Current.Name -eq 'BENCHMARKS'}).Count){break} }; Start-Sleep 4 }
$all+=Dump $els 'home'
$tab=$els|?{$_.Current.Name -eq 'BENCHMARKS' -and $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Hyperlink'}|select -First 1
if($tab){ try{ $tab.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); $all+='invoked' }catch{ $all+="invoke failed: $($_.Exception.Message)" }; Start-Sleep 8; $w=Win; $els=Els $w; $all+=Dump $els 'benchmarks' }
$all | Set-Content -Encoding UTF8 C:\kfhw\probe2.txt
