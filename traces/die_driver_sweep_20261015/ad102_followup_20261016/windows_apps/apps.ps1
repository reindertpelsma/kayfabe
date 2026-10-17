$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$ok=$false
$root='C:\kf'
"APP_START utc=$([DateTime]::UtcNow.ToString('o'))" | Set-Content "$root\apps.start"
try {
 $gpu=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
 if($gpu.Count -ne 1 -or $gpu[0].DriverVersion -ne '32.0.15.8088' -or $gpu[0].ConfigManagerErrorCode -ne 0) { throw 'Apps require actual580.88 Code0' }
 $null=Get-WinEvent -LogName System -MaxEvents 1 -ErrorAction Stop
 $since=Get-Date
 $parts=@(Get-ChildItem "$root\downloads\blender.part*" | Sort-Object Name)
 if($parts.Count -ne 96) { throw 'Require all96bounded Blender chunks' }
 $zip="$root\downloads\blender.zip"
 $output=[IO.File]::Open($zip,[IO.FileMode]::Create,[IO.FileAccess]::Write)
 try {
  for($i=0;$i -lt 96;$i++) {
   if($parts[$i].Name -ne ('blender.part{0:d3}' -f $i)) { throw 'Chunk sequence mismatch' }
   $chunkStream=[IO.File]::OpenRead($parts[$i].FullName)
   try { $chunkStream.CopyTo($output) } finally { $chunkStream.Dispose() }
  }
 } finally { $output.Dispose() }
 if((Get-FileHash $zip).Hash.ToLower() -ne '2ee75e9466d293a784fdf020f60fe1309c1e0610ecf73c64f1fc09b01e5eec56') { throw 'Pinned Blender ZIP hash mismatch' }
 Write-Output 'WINDOWS_BLENDER_ZIP_PIN_PASS=1'
 & tar.exe -xf $zip -C C:\kfapps\blender
 if($LASTEXITCODE) { throw 'Blender extraction failed' }
 $blender='C:\kfapps\blender\blender-4.5.0-windows-x64\blender.exe'
 $blenderArgs=@('-b','--factory-startup','--python-exit-code','1','--python','C:\kf\blender_render.py','--','CUDA','C:\kf\out\blender-cuda.png')
 $p=Start-Process $blender -ArgumentList $blenderArgs -PassThru -RedirectStandardOutput "$root\out\blender-cuda.log" -RedirectStandardError "$root\out\blender-cuda.err"
 $null=$p.Handle
 if(!$p.WaitForExit(180000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; throw 'Blender CUDA exceeded180seconds' }
 $p.Refresh()
 Write-Output "WINDOWS_BLENDER_PROCESS_EXIT code=$($p.ExitCode)"
 $stdout=Get-Content -Raw "$root\out\blender-cuda.log"
 Write-Output $stdout
 if($p.ExitCode -ne 0 -or $stdout -notmatch 'BLENDER_OK CUDA gpu=NVIDIA') { throw "Blender CUDA failed rc=$($p.ExitCode)" }
 $image=Get-Item "$root\out\blender-cuda.png"
 Write-Output "WINDOWS_BLENDER_CUDA_PASS bytes=$($image.Length) sha256=$((Get-FileHash $image.FullName).Hash.ToLower())"
 # Edge runs in the signed-in desktop. Its page checks NVIDIA renderer and pixels.
 $user=(Get-CimInstance Win32_ComputerSystem).UserName
 if(!$user) { throw 'Edge requires the signed-in desktop user' }
 $task='CodexAdaShortEdge'
 $action=New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -ExecutionPolicy Bypass -File C:\kf\edge_short_task.ps1'
 $principal=New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
 $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 3) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
 Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Settings $settings -Force | Out-Null
 Remove-Item "$root\out\edge.exit" -ErrorAction SilentlyContinue
 Start-ScheduledTask $task
 $deadline=(Get-Date).AddSeconds(120)
 while(!(Test-Path "$root\out\edge.exit") -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }
 if(!(Test-Path "$root\out\edge.exit")) { throw 'Edge short supervisor deadline exceeded' }
 Get-Content "$root\out\edge.log"
 if((Get-Content -Raw "$root\out\edge.exit").Trim() -ne '0') { throw 'Edge short failed' }
 $result=Get-Content -Raw "$root\out\codex_adaref2_webgl\edge_result.json" | ConvertFrom-Json
 if(!$result.ok -or $result.software -or $result.renderer -notmatch 'NVIDIA') { throw 'Edge did not prove hardware WebGL' }
 Write-Output 'WINDOWS_EDGE_SHORT_PASS=1'
 try { $events=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$since} -ErrorAction Stop) }
 catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw }; $events=@() }
 Write-Output "WINDOWS_APPS_TDR_COUNT=$($events.Count)"
 if($events.Count) { throw 'TDR during short apps' }
 $ok=$true
 Write-Output 'WINDOWS_SHORT_APPS_PASS=1'
} finally {
 "APP_EXIT rc=$([int](!$ok)) utc=$([DateTime]::UtcNow.ToString('o'))" | Set-Content "$root\apps.exit"
}
