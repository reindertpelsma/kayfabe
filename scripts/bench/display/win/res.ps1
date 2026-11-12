param([int]$W,[int]$H)
Add-Type @"
using System; using System.Runtime.InteropServices;
public class KfRes {
 [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct DEVMODE {
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName; public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields;
  public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput; public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName; public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency, dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight; }
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumDisplaySettings(string dn, int m, ref DEVMODE d);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int ChangeDisplaySettings(ref DEVMODE d, int f);
}
"@
$d = New-Object KfRes+DEVMODE; $d.dmSize=[int][Runtime.InteropServices.Marshal]::SizeOf($d)
[void][KfRes]::EnumDisplaySettings([NullString]::Value,-1,[ref]$d); "current $($d.dmPelsWidth)x$($d.dmPelsHeight)@$($d.dmDisplayFrequency)"
if($W -gt 0){ $d.dmPelsWidth=$W; $d.dmPelsHeight=$H; $d.dmFields=0x180000; "CDS=" + [KfRes]::ChangeDisplaySettings([ref]$d,0) }
[void][KfRes]::EnumDisplaySettings([NullString]::Value,-1,[ref]$d); "now $($d.dmPelsWidth)x$($d.dmPelsHeight)"
$i=0; $m=New-Object KfRes+DEVMODE; $m.dmSize=$d.dmSize; $l=@(); while([KfRes]::EnumDisplaySettings([NullString]::Value,$i,[ref]$m)){ $l+="$($m.dmPelsWidth)x$($m.dmPelsHeight)"; $i++ }; "modes: " + (($l | sort -Unique) -join ' ')
