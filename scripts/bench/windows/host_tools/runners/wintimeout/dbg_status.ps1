Get-Content C:\kfdbg\install.log -ErrorAction SilentlyContinue
Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe' -ErrorAction SilentlyContinue | ForEach-Object { "cdb " + $_.FullName }
Get-ChildItem C:\Windows\MEMORY.DMP, C:\Windows\Minidump\* -ErrorAction SilentlyContinue | ForEach-Object { "dump " + $_.FullName + " " + $_.Length }
