# kfocc.ps1: an opaque topmost cmd window over the video's lower middle (logical px; the guest is DPI-scaled 1.25)
Add-Type @"
using System; using System.Runtime.InteropServices;
public class KfW { [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f); }
"@
$p = Start-Process cmd.exe -ArgumentList '/k','title kf-occluder' -PassThru
Start-Sleep -Seconds 3
$p.Refresh()
[KfW]::SetWindowPos($p.MainWindowHandle, [IntPtr](-1), 700, 300, 250, 300, 0x40) | Out-File C:\kf\occ.txt
