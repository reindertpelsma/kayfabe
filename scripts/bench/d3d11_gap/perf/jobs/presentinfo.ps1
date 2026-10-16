# job (service or interactive): how is Heaven presenting right now? window geometry/style, DWM state, process CPU. Read-only.
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("dwmapi.dll")] public static extern int DwmIsCompositionEnabled(out bool e);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
$fg = [W]::GetForegroundWindow()
$e = $false; [void][W]::DwmIsCompositionEnabled([ref]$e); "DWM composition enabled: $e"
$pids = @{}; Get-Process -Name Heaven,dwm,csrss -ErrorAction SilentlyContinue | ForEach-Object { $pids[[uint32]$_.Id] = $_.ProcessName }
$cb = [W+EnumProc]{ param($h, $l)
  $p = 0; [void][W]::GetWindowThreadProcessId($h, [ref]$p)
  if ($pids.ContainsKey([uint32]$p) -and [W]::IsWindowVisible($h)) {
    $r = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$r)
    $sb = New-Object System.Text.StringBuilder 128; [void][W]::GetWindowText($h, $sb, 128)
    $st = [W]::GetWindowLong($h, -16); $ex = [W]::GetWindowLong($h, -20)
    "WIN pid=$p $($pids[[uint32]$p]) title='$($sb.ToString())' rect=$($r.L),$($r.T),$($r.R),$($r.B) style=0x$('{0:x}' -f $st) exstyle=0x$('{0:x}' -f $ex) foreground=$($h -eq $fg)"
  }
  return $true }
[void][W]::EnumWindows($cb, [IntPtr]::Zero)
Add-Type -AssemblyName System.Windows.Forms
foreach ($s in [System.Windows.Forms.Screen]::AllScreens) { "SCREEN $($s.DeviceName) bounds=$($s.Bounds) primary=$($s.Primary)" }
$a = Get-Process -Name Heaven,dwm -ErrorAction SilentlyContinue | Select-Object Name, Id, @{n='cpu';e={$_.CPU}}
Start-Sleep 5
$b = Get-Process -Name Heaven,dwm -ErrorAction SilentlyContinue | Select-Object Name, Id, @{n='cpu';e={$_.CPU}}
foreach ($x in $b) { $y = $a | Where-Object { $_.Id -eq $x.Id }; if ($y) { "PROC $($x.Name) pid=$($x.Id) cpu_s_in_5s=$([math]::Round($x.cpu - $y.cpu, 2))" } }
try { Get-Counter '\Process(dwm)\% Processor Time','\Process(Heaven)\% Processor Time' -SampleInterval 1 -MaxSamples 3 | ForEach-Object { $_.CounterSamples | ForEach-Object { "CTR $($_.Path) $([math]::Round($_.CookedValue,1))" } } } catch { "CTR fail $_" }
"HDR " + ((& "$env:SystemRoot\System32\nvidia-smi.exe" --query-gpu=driver_version,pstate,clocks.sm,clocks.mem,power.draw --format=csv,noheader) -join ';')
