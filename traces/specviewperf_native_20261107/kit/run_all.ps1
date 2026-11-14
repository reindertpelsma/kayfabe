# run_all.ps1 -List a,b,c -- run spec_run.ps1 for each viewset in turn (one CLI run each), in the interactive session; continues past failures.
param([Parameter(Mandatory = $true)][string]$List)
foreach ($w in $List.Split(',')) {
    "QUEUE_BEGIN $w " + (Get-Date).ToUniversalTime().ToString('o') | Add-Content C:\spec\queue.log
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\spec_run.ps1 -Workload $w
    "QUEUE_DONE $w " + (Get-Date).ToUniversalTime().ToString('o') + ' ' + (Get-Content "C:\spec\out\$w\END.txt" -ErrorAction SilentlyContinue) | Add-Content C:\spec\queue.log
}
'QUEUE_ALL_DONE ' + (Get-Date).ToUniversalTime().ToString('o') | Add-Content C:\spec\queue.log
