param([string]$Out="C:\kf\t\shot.png")
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @"
using System.Runtime.InteropServices; public class D{[DllImport("user32.dll")]public static extern bool SetProcessDPIAware();}
"@
[D]::SetProcessDPIAware() | Out-Null
$b=[System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height
$g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size)
$bmp.Save($Out,[System.Drawing.Imaging.ImageFormat]::Png); "shot $($b.Width)x$($b.Height)"
