# gm.ps1 -Api d3d12|vk|d3d11|gl -Tag <t> : one GravityMark 1.89 benchmark pass (the app matrix's arguments), image + times
param([string]$Api = 'd3d12', [string]$Tag = 'gm', [int]$Fs = 0)
$o = "C:\kf\apps\out"; New-Item -ItemType Directory -Force $o | Out-Null
$f = "$o\$Tag.txt"; $gmx = 'C:\kf\apps\gravitymark\GravityMark\bin\GravityMark.exe'
"START $Tag api=$Api $((Get-Date).ToString('HH:mm:ss.fff'))" | Set-Content $f
$a = @('-temporal','1','-fps','1','-info','1','-benchmark','1','-close','1','-count','1','-asteroids','20000',
  '-width','1280','-height','720','-fullscreen',"$Fs","-$Api",'-image',"$o\$Tag.png",'-times',"$o\$Tag.times.txt")
$p = Start-Process -FilePath $gmx -ArgumentList $a -WorkingDirectory (Split-Path $gmx) -PassThru `
  -RedirectStandardOutput "$o\$Tag.stdout.txt" -RedirectStandardError "$o\$Tag.stderr.txt"
if (-not $p.WaitForExit(660000)) { "TIMEOUT" | Add-Content $f; $p.Kill() }
"END rc=$($p.ExitCode) $((Get-Date).ToString('HH:mm:ss.fff'))" | Add-Content $f
