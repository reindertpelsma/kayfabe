# mc.ps1: start Minecraft Java with the command line the launcher used (saved in C:\kf\ogl\mc_cmd.txt; never printed)
$c = (Get-Content C:\kf\ogl\mc_cmd.txt -Raw).Trim()
if ($c.StartsWith('"')) { $i = $c.IndexOf('"', 1); $exe = $c.Substring(1, $i - 1); $rest = $c.Substring($i + 1).Trim() }
else { $i = $c.IndexOf(' '); $exe = $c.Substring(0, $i); $rest = $c.Substring($i + 1) }
$p = Start-Process -FilePath $exe -ArgumentList $rest -WorkingDirectory "$env:APPDATA\.minecraft" -PassThru
"MC STARTED pid=$($p.Id) at $((Get-Date).ToString('HH:mm:ss.fff'))" | Set-Content C:\kf\ogl\mc_start.txt
