# heaven_job.ps1 -Api direct3d11|opengl -Label NAME [-OutRoot C:\bench\runs] : ONE Heaven 4.0 in-app benchmark in the INTERACTIVE session.
# Mirrors origin/claude/perf-win-20261015 scripts/bench/windows/perf/jobs/heaven.ps1.in (same arguments), plus: dmon sampler for the
# whole run, nvidia-smi -q at idle and under load, a mid-run screenshot, display facts. Windowed 1920x1080, click + F9 to start the benchmark.
param([Parameter(Mandatory)][ValidateSet('direct3d11','opengl')][string]$Api, [Parameter(Mandatory)][string]$Label,
      [string]$OutRoot = 'C:\bench\runs', [string]$Defs = 'RELEASE,QUALITY_ULTRA', [int]$LoadSnapAfter = 100)
$ErrorActionPreference = 'Continue'
$out = Join-Path $OutRoot $Label; New-Item -ItemType Directory -Force $out | Out-Null
$ev = Join-Path $out 'events.txt'
function Ev($s) { "$((Get-Date).ToString('HH:mm:ss.fff')) $s" | Add-Content $ev }
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace W -Name U -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
'@
function Click($x, $y) { [W.U]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 200; [W.U]::mouse_event(2,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 80; [W.U]::mouse_event(4,0,0,0,[UIntPtr]::Zero) }
function F9() { [W.U]::keybd_event(0x78,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 80; [W.U]::keybd_event(0x78,0,2,[UIntPtr]::Zero) }
function Shot($name) { $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $bm = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bm); $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size); $bm.Save((Join-Path $out $name), [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bm.Dispose() }
$h = 'C:\kfapps\heaven\bin'; $lg = Join-Path $env:USERPROFILE 'Heaven\log.html'
Remove-Item $lg -ErrorAction SilentlyContinue
Ev "START label=$Label api=$Api user=$env:USERNAME session=$((Get-Process -Id $PID).SessionId) defs=$Defs"
Add-Type -AssemblyName System.Windows.Forms
$scr = [System.Windows.Forms.Screen]::PrimaryScreen; Ev "PRIMARY_SCREEN $($scr.DeviceName) bounds=$($scr.Bounds)"
Get-CimInstance Win32_VideoController | ForEach-Object { Ev "VIDEO $($_.Name) mode=$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution)@$($_.CurrentRefreshRate)" }
& nvidia-smi -q -d PERFORMANCE,CLOCK,POWER,PIDS 2>&1 | Set-Content (Join-Path $out 'smi_q_idle.txt')
& nvidia-smi --query-gpu=pstate,clocks.sm,clocks.mem,power.draw,utilization.gpu,pcie.link.gen.current,pcie.link.width.current,temperature.gpu --format=csv 2>&1 | Set-Content (Join-Path $out 'smi_query_idle.txt')
$dm = Start-Process nvidia-smi -ArgumentList 'dmon','-s','pucvmet','-d','1','-o','T' -RedirectStandardOutput (Join-Path $out 'dmon.txt') -PassThru -WindowStyle Hidden
Start-Sleep 5
$a = "-project_name Heaven -data_path ../ -engine_config ../data/heaven_4.0.cfg -system_script heaven/unigine.cpp -sound_app null -video_app $Api -video_multisample 0 -video_fullscreen 0 -video_mode -1 -video_width 1920 -video_height 1080 -extern_define $Defs"
$t0 = Get-Date; Ev "LAUNCH $a"
$p = Start-Process "$h\Heaven.exe" -ArgumentList $a -WorkingDirectory $h -PassThru
# wait for the window and the engine's start-up
$hw = [IntPtr]::Zero; for ($i = 0; $i -lt 90 -and -not $p.HasExited; $i++) { Start-Sleep 1; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $hw = $p.MainWindowHandle; if ($i -gt 20) { break } } }
Ev "WINDOW hwnd=$hw exited=$($p.HasExited) after=$([int]((Get-Date)-$t0).TotalSeconds)s"
Start-Sleep 12
$r = New-Object W.U+RECT; [W.U]::GetWindowRect($hw, [ref]$r) | Out-Null
Ev "WINRECT $($r.L),$($r.T),$($r.R),$($r.B) w=$($r.R-$r.L) h=$($r.B-$r.T) style=0x$([W.U]::GetWindowLong($hw,-16).ToString('x'))"
$cx = [int](($r.L + $r.R) / 2); $cy = [int](($r.T + $r.B) / 2); if ($cx -lt 0 -or $cx -gt $scr.Bounds.Width) { $cx = [int]($scr.Bounds.Width/2) }; if ($cy -lt 0 -or $cy -gt $scr.Bounds.Height) { $cy = [int]($scr.Bounds.Height/2) }
Shot 'shot_preF9.png'
[W.U]::SetForegroundWindow($hw) | Out-Null; Click $cx $cy; Start-Sleep 1; F9; $f9 = Get-Date; Ev "F9 sent at x=$cx y=$cy"
for ($try = 1; $try -le 3; $try++) { Start-Sleep 8; if ((Test-Path $lg) -and (Select-String -Path $lg -Pattern 'Benchmark running' -Quiet)) { break }; Click $cx $cy; Start-Sleep 1; F9; Ev "F9 retry $try" }
$snapped = $false; $found = $false
while (((Get-Date) - $t0).TotalSeconds -lt 560 -and -not $p.HasExited) {
  Start-Sleep 2
  if (-not $snapped -and ((Get-Date) - $f9).TotalSeconds -gt $LoadSnapAfter) { $snapped = $true
    & nvidia-smi -q -d PERFORMANCE,CLOCK,POWER,PIDS 2>&1 | Set-Content (Join-Path $out 'smi_q_load.txt')
    & nvidia-smi --query-gpu=pstate,clocks.sm,clocks.mem,power.draw,utilization.gpu,pcie.link.gen.current,pcie.link.width.current,temperature.gpu --format=csv 2>&1 | Set-Content (Join-Path $out 'smi_query_load.txt')
    Shot 'shot_load.png'; Ev 'LOAD_SNAP done' }
  if ((Test-Path $lg) -and (Select-String -Path $lg -Pattern 'Benchmark results' -Quiet)) { $found = $true; break }
}
Ev "RESULT_FOUND=$found alive=$(-not $p.HasExited) elapsed_s=$([int]((Get-Date)-$t0).TotalSeconds)"
Start-Sleep 3
if (Test-Path $lg) { Copy-Item $lg (Join-Path $out 'heaven_log.html'); Get-Content $lg | Where-Object { $_ -match 'Benchmark|Time:|Frames:|FPS|Score|Direct3D11 desc|OpenGL|Renderer|GL_RENDERER|desc:' } | ForEach-Object { 'HLOG ' + ($_ -replace '<[^>]*>','' -replace '&nbsp;',' ') } | Add-Content $ev }
if (-not $p.HasExited) { $p.Kill() }
Start-Sleep 3
if (-not $dm.HasExited) { $dm.Kill() }
Ev 'END'
'done' | Set-Content (Join-Path $out 'DONE')
