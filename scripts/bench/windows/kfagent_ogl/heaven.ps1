# heaven.ps1 -Api direct3d11|opengl -Fs 0|1 -Tag <t> : one full Heaven 4.0 benchmark pass (PHORONIX define: run the
# benchmark sequence and exit), stdout to C:\kf\apps\out\<t>.txt, start/end/rc lines appended.
param([string]$Api = 'direct3d11', [int]$Fs = 0, [string]$Tag = 'heaven')
$o = "C:\kf\apps\out"; New-Item -ItemType Directory -Force $o | Out-Null
$f = "$o\$Tag.txt"
"START $Tag api=$Api fs=$Fs $((Get-Date).ToString('HH:mm:ss.fff'))" | Set-Content $f
$w = if ($Fs) { 1920 } else { 1280 }; $h = if ($Fs) { 1080 } else { 720 }
$args = @('-project_name','Heaven','-data_path','../','-engine_config','../data/heaven_4.0.cfg','-system_script','heaven/unigine.cpp',
  '-sound_app','null','-video_app',$Api,'-video_multisample','0','-video_fullscreen',"$Fs",'-video_mode','-1',
  '-video_width',"$w",'-video_height',"$h",'-extern_define','PHORONIX,RELEASE,LANGUAGE_EN,QUALITY_HIGH,TESSELLATION_NORMAL')
$p = Start-Process -FilePath C:\kf\apps\heaven\bin\Heaven.exe -ArgumentList $args -WorkingDirectory C:\kf\apps\heaven\bin -PassThru `
  -RedirectStandardOutput "$o\$Tag.stdout.txt" -RedirectStandardError "$o\$Tag.stderr.txt"
if (-not $p.WaitForExit(900000)) { "TIMEOUT" | Add-Content $f; $p.Kill() }
"END rc=$($p.ExitCode) $((Get-Date).ToString('HH:mm:ss.fff'))" | Add-Content $f
