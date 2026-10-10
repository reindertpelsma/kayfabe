$ErrorActionPreference = 'Continue'
$cdb = 'C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe'
$cmds = '.echo KF_RAW; dps ffff8081dc767010 L30; .echo KF_DXGKDX; !dxgkdx.help; .echo KF_STACKS; !stacks 2 dxgkrnl; .echo KF_STACKS2; !stacks 2 nvlddmkm; .echo KF_DONE; q'
Remove-Item C:\kfdbg\analyze.log -ErrorAction SilentlyContinue
Start-Process -FilePath $cdb -ArgumentList @('-z', 'C:\Windows\MEMORY.DMP', '-y', 'srv*c:\kfdbg\sym*https://msdl.microsoft.com/download/symbols', '-logo', 'C:\kfdbg\analyze.log', '-c', "`"$cmds`"") -WindowStyle Hidden
"launched"
