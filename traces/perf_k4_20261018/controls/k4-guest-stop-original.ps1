$ErrorActionPreference='Stop'
$root='Q4_GPU_ROOT'
$owned=@(Get-Content "$root\collectors-owned.json" -Raw|ConvertFrom-Json)
$result=@()
foreach($row in $owned){$p=Get-Process -Id $row.pid -ErrorAction SilentlyContinue;$stopped=$false;if($p){if($p.StartTime.ToFileTimeUtc() -ne $row.start -or $p.Path -ne $row.exe){throw 'COLLECTOR_PID_IDENTITY_CHANGED'};$p.Kill();$p.WaitForExit();$stopped=$true};$result+=@{name=$row.name;pid=$row.pid;stopped=$stopped;errors=([IO.File]::ReadAllText("$root\$($row.name)-error.txt"))}}
$result|ConvertTo-Json -Compress -Depth 4
