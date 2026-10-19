# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Adapted from the public hitch heaven_fs job; same CLI RELEASE,QUALITY_ULTRA.
param([ValidateSet('launch','benchmark','result','stop')][string]$Action='launch', [string]$Tag='heaven', [string]$Out='C:\kf\P1\heaven_ui.txt')
$ErrorActionPreference='Stop'
if ($Tag -notmatch '^[A-Za-z0-9_-]+$') { throw 'Unsafe tag' }
$r='C:\kf\P1'; $state="$r\${Tag}_heaven.json"
$exe='C:\kf\apps\heaven\bin\Heaven.exe'; $lg="$env:USERPROFILE\Heaven\log.html"
if ($Action -eq 'launch') {
  if (Get-Process Heaven -ErrorAction SilentlyContinue) { throw 'Existing Heaven process: no process signalled' }
  $args2='-project_name Heaven -data_path ../ -engine_config ../data/heaven_4.0.cfg -system_script heaven/unigine.cpp -sound_app null -video_app direct3d11 -video_multisample 0 -video_vsync 1 -video_fullscreen 1 -video_mode -1 -video_width 1920 -video_height 1080 -extern_define RELEASE,QUALITY_ULTRA'
  $p=Start-Process $exe -ArgumentList $args2 -WorkingDirectory (Split-Path $exe) -PassThru
  $s=@{pid=$p.Id;start=$p.StartTime.ToUniversalTime().ToString('o');args=$args2;log=$lg}
  $s | ConvertTo-Json | Set-Content $state
  $s | ConvertTo-Json | Set-Content $Out
  exit
}
$s=Get-Content $state -Raw | ConvertFrom-Json
$p=Get-Process -Id $s.pid -ErrorAction Stop
if ($p.StartTime.ToUniversalTime().ToString('o') -ne $s.start -or $p.Path -ne $exe) { throw 'Heaven ownership mismatch' }
if ($Action -eq 'benchmark') {
  Add-Type -AssemblyName System.Windows.Forms
  Add-Type -Namespace P1Heaven -Name W -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);'
  if (-not [P1Heaven.W]::SetForegroundWindow($p.MainWindowHandle)) { throw 'Heaven foreground refused' }
  Start-Sleep -Milliseconds 500
  [System.Windows.Forms.SendKeys]::SendWait('{F9}')
  ('P1_HEAVEN_F9 '+(Get-Date).ToUniversalTime().ToString('o')) | Set-Content $Out
}
if ($Action -eq 'result') {
  if ((Get-Item $lg).LastWriteTimeUtc -lt $p.StartTime.ToUniversalTime()) { throw 'Heaven log predates this process' }
  if (-not (Select-String -Path $lg -Pattern 'Benchmark results' -Quiet)) { throw 'Heaven benchmark incomplete' }
  Copy-Item $lg "$r\${Tag}_heaven.html"
  Get-Content $lg | Where-Object { $_ -match 'Benchmark|Time:|Frames:|FPS|Score|Direct3D11 desc|vsync|VSync' } | ForEach-Object { 'HLOG '+($_ -replace '<[^>]*>','' -replace '&nbsp;',' ') } | Set-Content $Out
}
if ($Action -eq 'stop') {
  [void]$p.CloseMainWindow()
  if (-not $p.WaitForExit(15000)) { $p.Kill(); $p.WaitForExit() }
  'P1_HEAVEN_STOPPED' | Set-Content $Out
}
