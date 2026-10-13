param([string]$Out,[string]$Extra="",[int]$Secs)
New-Item -ItemType Directory -Force $Out | Out-Null
$cmd = "C:\kfwc\build-out\kfwc_collect.exe $Out --seconds $Secs --decode-tag 580.82.07 $Extra > $Out.stdout.txt 2> $Out.stderr.txt"
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine="cmd.exe /c $cmd"}
"create rc=$($r.ReturnValue) pid=$($r.ProcessId)"
