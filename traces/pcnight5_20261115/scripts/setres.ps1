Add-Type @'
using System; using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Sequential, CharSet=CharSet.Ansi)]
public struct DEVMODE { [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName; public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields; public int dmPositionX, dmPositionY; public int dmDisplayOrientation, dmDisplayFixedOutput; public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName; public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency, dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight; }
public class KfRes { [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern int EnumDisplaySettings(string d,int m,ref DEVMODE dm); [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern int ChangeDisplaySettings(ref DEVMODE dm,int f); }
'@
$dm=New-Object DEVMODE; $dm.dmSize=[int][System.Runtime.InteropServices.Marshal]::SizeOf($dm)
[void][KfRes]::EnumDisplaySettings($null,-1,[ref]$dm)
"before: $($dm.dmPelsWidth)x$($dm.dmPelsHeight) @$($dm.dmDisplayFrequency)" | Set-Content C:\kfhw\setres.txt
$dm.dmPelsWidth=1920; $dm.dmPelsHeight=1080; $dm.dmFields=0x80000 -bor 0x100000
$r=[KfRes]::ChangeDisplaySettings([ref]$dm,1)
[void][KfRes]::EnumDisplaySettings($null,-1,[ref]$dm)
"result=$r after: $($dm.dmPelsWidth)x$($dm.dmPelsHeight)" | Add-Content C:\kfhw\setres.txt
