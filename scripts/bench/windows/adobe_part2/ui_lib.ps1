# Guest-side desktop helpers for the Adobe part-2 run (PowerShell 5.1, interactive session, normal integrity). Prepended after lib.ps1 by drive.py.
# Screenshots are taken of the whole virtual screen, scaled to $w pixels wide and saved as PNG in $D; the host looks at them and decides the next action.
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfU {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern void mouse_event(int f, int dx, int dy, int d, int e);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
}
'@
[void][KfU]::SetProcessDPIAware()
function Shot([string]$name, [int]$w = 1280) {
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
  $h = [int]($b.Height * $w / $b.Width)
  $small = New-Object System.Drawing.Bitmap $bmp, $w, $h
  $small.Save("$D\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose(); $small.Dispose()
  return ('{0}x{1}->{2}x{3}' -f $b.Width, $b.Height, $w, $h)
}
function Move-To($x, $y) { [void][KfU]::SetCursorPos([int]$x, [int]$y); Start-Sleep -Milliseconds 150 }
function Click-At($x, $y) { Move-To $x $y; [KfU]::mouse_event(2, 0, 0, 0, 0); Start-Sleep -Milliseconds 60; [KfU]::mouse_event(4, 0, 0, 0, 0); Start-Sleep -Milliseconds 300 }
function DClick-At($x, $y) { Click-At $x $y; Click-At $x $y }
function RClick-At($x, $y) { Move-To $x $y; [KfU]::mouse_event(8, 0, 0, 0, 0); Start-Sleep -Milliseconds 60; [KfU]::mouse_event(16, 0, 0, 0, 0); Start-Sleep -Milliseconds 300 }
function Drag-To($x1, $y1, $x2, $y2) {
  Move-To $x1 $y1; [KfU]::mouse_event(2, 0, 0, 0, 0); Start-Sleep -Milliseconds 200
  $n = 12
  for ($i = 1; $i -le $n; $i++) { [void][KfU]::SetCursorPos([int]($x1 + ($x2 - $x1) * $i / $n), [int]($y1 + ($y2 - $y1) * $i / $n)); Start-Sleep -Milliseconds 40 }
  Start-Sleep -Milliseconds 200; [KfU]::mouse_event(4, 0, 0, 0, 0); Start-Sleep -Milliseconds 300
}
# Bring the main window of a process to the foreground.
function Focus-Proc([string]$proc) {
  $p = Get-Process $proc -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if ($p) { [void][KfU]::ShowWindow($p.MainWindowHandle, 9); [void][KfU]::SetForegroundWindow($p.MainWindowHandle); Start-Sleep -Milliseconds 400; return $true }
  return $false
}
# Top-level window titles of every process (name | title), for a sign-in prompt search that does not depend on the application's own process names.
function All-Titles {
  $roots = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
  @(foreach ($r in $roots) { try { $pr = Get-Process -Id $r.Current.ProcessId -ErrorAction SilentlyContinue; if ($r.Current.Name) { ('{0} | {1}' -f $pr.ProcessName, $r.Current.Name) } } catch {} })
}
