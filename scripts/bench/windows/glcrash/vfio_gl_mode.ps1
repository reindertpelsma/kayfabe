# vfio_gl_mode.ps1 W H [HZ] -- QGA (SYSTEM): set the primary display mode in the signed-in user's interactive session (scheduled task /it),
# print the resulting mode. Used by the 2026-10-11 VFIO GL reference to see how the real GSP's 0x73011a answer follows the mode.
param([int]$W = 1280, [int]$H = 720, [int]$Hz = 60)
$u = @"
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public class Disp {
  [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern int EnumDisplaySettings(string d, int m, IntPtr dm);
  [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern int ChangeDisplaySettingsEx(string d, IntPtr dm, IntPtr h, int fl, IntPtr p);
  public static string Cur(IntPtr p) { return Marshal.ReadInt32(p, 108) + "x" + Marshal.ReadInt32(p, 112) + "@" + Marshal.ReadInt32(p, 120); }
  public static string Set(int w, int h, int hz) {
    IntPtr p = Marshal.AllocHGlobal(156); for (int i = 0; i < 156; i += 4) Marshal.WriteInt32(p, i, 0);
    Marshal.WriteInt16(p, 36, 156);
    int e = EnumDisplaySettings(null, -1, p); string before = Cur(p) + " enum=" + e;
    Marshal.WriteInt32(p, 108, w); Marshal.WriteInt32(p, 112, h); Marshal.WriteInt32(p, 120, hz); Marshal.WriteInt32(p, 40, 0x80000 | 0x100000 | 0x400000);
    int r = ChangeDisplaySettingsEx(null, p, IntPtr.Zero, 1, IntPtr.Zero);
    System.Threading.Thread.Sleep(3000);
    EnumDisplaySettings(null, -1, p);
    return "before " + before + " change=" + r + " now " + Cur(p);
  }
}
'@
[Disp]::Set($W, $H, $Hz) | Set-Content C:\kf\mode-result.txt
'DONE' | Add-Content C:\kf\mode-result.txt
"@
New-Item -ItemType Directory -Force -Path C:\kf | Out-Null
Set-Content -Path C:\kf\vgl_mode.ps1 -Value $u -Encoding UTF8
Remove-Item C:\kf\mode-result.txt -ErrorAction SilentlyContinue
$user = (Get-Process explorer -ErrorAction SilentlyContinue | Select-Object -First 1).UserName
if (-not $user) { $user = "$env:COMPUTERNAME\vast" }
schtasks /delete /tn kfvglmode /f 2>&1 | Out-Null
schtasks /create /tn kfvglmode /tr 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\vgl_mode.ps1' /sc once /st 00:00 /it /ru $user /rl highest /f 2>&1 | Out-Null
schtasks /run /tn kfvglmode 2>&1 | Out-Null
for ($i = 0; $i -lt 60 -and -not ((Test-Path C:\kf\mode-result.txt) -and (Select-String -Path C:\kf\mode-result.txt -Pattern DONE -Quiet)); $i++) { Start-Sleep 1 }
schtasks /delete /tn kfvglmode /f 2>&1 | Out-Null
Get-Content C:\kf\mode-result.txt -ErrorAction SilentlyContinue
