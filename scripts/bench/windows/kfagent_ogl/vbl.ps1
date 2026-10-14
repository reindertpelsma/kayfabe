param([int]$N = 120, [string]$Out = 'C:\kf\ogl\vbl.txt')
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class Kmt {
 [StructLayout(LayoutKind.Sequential)] public struct OPENFROMHDC { public IntPtr hDc; public uint hAdapter; public uint LuidLow; public int LuidHigh; public uint VidPnSourceId; }
 [StructLayout(LayoutKind.Sequential)] public struct WAITVBL { public uint hAdapter; public uint hDevice; public uint VidPnSourceId; }
 [DllImport("gdi32.dll")] public static extern int D3DKMTOpenAdapterFromHdc(ref OPENFROMHDC p);
 [DllImport("gdi32.dll")] public static extern int D3DKMTWaitForVerticalBlankEvent(ref WAITVBL p);
 [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
}
"@
$o = New-Object Kmt+OPENFROMHDC; $o.hDc = [Kmt]::GetDC([IntPtr]::Zero)
$r = [Kmt]::D3DKMTOpenAdapterFromHdc([ref]$o)
"VBL open r=$r hAdapter=$($o.hAdapter) luid=$($o.LuidHigh):$($o.LuidLow) src=$($o.VidPnSourceId) at $((Get-Date).ToString('HH:mm:ss.fff'))" | Set-Content $Out
$w = New-Object Kmt+WAITVBL; $w.hAdapter = $o.hAdapter; $w.VidPnSourceId = $o.VidPnSourceId
$sw = [Diagnostics.Stopwatch]::StartNew(); $max = 0.0; $last = 0.0
for ($i = 1; $i -le $N; $i++) {
  if ($i -le 3) { "VBL waiting #$i at $((Get-Date).ToString('HH:mm:ss.fff'))" | Add-Content $Out }
  $r = [Kmt]::D3DKMTWaitForVerticalBlankEvent([ref]$w)
  $t = $sw.Elapsed.TotalMilliseconds; $d = $t - $last; $last = $t; if ($d -gt $max) { $max = $d }
  if ($r -ne 0 -or $i % 30 -eq 0) { "VBL #$i r=$r t_ms=$([int]$t) max_gap_ms=$([int]$max)" | Add-Content $Out }
}
"VBL DONE n=$N ms=$([int]$sw.Elapsed.TotalMilliseconds) hz=$([math]::Round($N*1000/$sw.Elapsed.TotalMilliseconds,1)) max_gap_ms=$([int]$max)" | Add-Content $Out
