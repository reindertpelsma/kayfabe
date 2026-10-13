function cpu { (Get-Process -Id 4).CPU }
$a=cpu; Start-Sleep 30; $b=cpu; "baseline System CPU s per s: " + (($b-$a)/30)
& powershell -NoProfile -ExecutionPolicy Bypass -File C:\kfwc\start.ps1 -Out C:\traces\H -Extra "--no-mailbox" -Secs 80 | Out-Null
Start-Sleep 20
$a=cpu; Start-Sleep 40; $b=cpu; "collector attached (scan found by ~10 s), System CPU s per s over 40 s: " + (($b-$a)/40)
Start-Sleep 25
Get-Content C:\traces\H.stdout.txt
Get-WinEvent -FilterHashtable @{LogName='System';StartTime=(Get-Date).AddHours(-2)} -ea 0 | ? { $_.ProviderName -match 'nvlddmkm|Display|Kernel-Power|WHEA|BugCheck|volmgr' -or $_.Id -in 41,1001,4101,14 } | % { "{0} {1} {2}" -f $_.TimeCreated.ToString('HH:mm:ss'),$_.ProviderName,$_.Id }
"end"
