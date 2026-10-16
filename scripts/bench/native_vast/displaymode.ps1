# displaymode.ps1 [-Set] : list the primary display's modes and (with -Set) switch it to 1920x1080 (runs in the interactive session). Output C:\bench\displaymode.txt
param([switch]$Set, [int]$W = 1920, [int]$H = 1080)
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public class Dm {
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Ansi)] public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName; public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields;
    public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput; public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName; public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight; }
  [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern bool EnumDisplaySettings(string dev, int n, ref DEVMODE m);
  [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern int ChangeDisplaySettingsEx(string dev, ref DEVMODE m, IntPtr hwnd, int flags, IntPtr lp);
}
'@
$o = @()
$m = New-Object Dm+DEVMODE; $m.dmSize = [int16][Runtime.InteropServices.Marshal]::SizeOf($m)
$null = [Dm]::EnumDisplaySettings("\\.\DISPLAY1", -1, [ref]$m); $o += "CURRENT $($m.dmPelsWidth)x$($m.dmPelsHeight)@$($m.dmDisplayFrequency) bpp=$($m.dmBitsPerPel)"
$seen = @{}; $i = 0; $target = $null
while ([Dm]::EnumDisplaySettings("\\.\DISPLAY1", $i, [ref]$m)) { $k = "$($m.dmPelsWidth)x$($m.dmPelsHeight)@$($m.dmDisplayFrequency)"; if (-not $seen[$k]) { $seen[$k] = 1; $o += "MODE $k bpp=$($m.dmBitsPerPel)" }
  if ($m.dmPelsWidth -eq $W -and $m.dmPelsHeight -eq $H -and ($null -eq $target -or $m.dmDisplayFrequency -gt $target.dmDisplayFrequency)) { $target = $m }; $i++ }
if ($Set) { if ($target) { $r = [Dm]::ChangeDisplaySettingsEx("\\.\DISPLAY1", [ref]$target, [IntPtr]::Zero, 1, [IntPtr]::Zero); $o += "SET $W x $H rc=$r (0=OK, 1=restart needed)" } else { $o += "NO_MODE_${W}x$H" } }
$null = [Dm]::EnumDisplaySettings("\\.\DISPLAY1", -1, [ref]$m); $o += "NOW $($m.dmPelsWidth)x$($m.dmPelsHeight)@$($m.dmDisplayFrequency)"
$o | Set-Content C:\bench\displaymode.txt
