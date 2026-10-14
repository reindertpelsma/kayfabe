# runi.ps1 -Name <tag> -Cmd <cmdline> : run a command in kfadmin's interactive session via a one-shot scheduled task
param([string]$Name, [string]$Cmd)
$a = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument ('/c ' + $Cmd)
$p = New-ScheduledTaskPrincipal -UserId 'kfadmin' -LogonType Interactive -RunLevel Limited
Unregister-ScheduledTask -TaskName "kfogl-$Name" -Confirm:$false -ErrorAction SilentlyContinue
Register-ScheduledTask -TaskName "kfogl-$Name" -Action $a -Principal $p | Out-Null
Start-ScheduledTask -TaskName "kfogl-$Name"
"STARTED $Name " + (Get-Date).ToString('HH:mm:ss.fff')
