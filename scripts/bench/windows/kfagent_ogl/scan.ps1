param([int]$N = 400, [int]$SleepMs = 2, [string]$Out = 'C:\kf\ogl\scan.txt')
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class KmtS {
 [StructLayout(LayoutKind.Sequential)] public struct OPENFROMHDC { public IntPtr hDc; public uint hAdapter; public uint LuidLow; public int LuidHigh; public uint VidPnSourceId; }
 [StructLayout(LayoutKind.Sequential)] public struct GETSCANLINE { public uint hAdapter; public uint VidPnSourceId; public byte InVerticalBlank; public uint ScanLine; }
 [DllImport("gdi32.dll")] public static extern int D3DKMTOpenAdapterFromHdc(ref OPENFROMHDC p);
 [DllImport("gdi32.dll")] public static extern int D3DKMTGetScanLine(ref GETSCANLINE p);
 [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
}
"@
$o = New-Object KmtS+OPENFROMHDC; $o.hDc = [KmtS]::GetDC([IntPtr]::Zero); $r = [KmtS]::D3DKMTOpenAdapterFromHdc([ref]$o)
"SCAN open r=$r src=$($o.VidPnSourceId) at $((Get-Date).ToString('HH:mm:ss.fff'))" | Set-Content $Out
$g = New-Object KmtS+GETSCANLINE; $g.hAdapter = $o.hAdapter; $g.VidPnSourceId = $o.VidPnSourceId
$lines = @{}; $vb = 0; $err = 0; $first = ''; $sw = [Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt $N; $i++) {
  $r = [KmtS]::D3DKMTGetScanLine([ref]$g); if ($r -ne 0) { $err++; if (-not $first) { $first = "r=0x{0:x}" -f $r } }
  $lines[$g.ScanLine] = 1 + [int]$lines[$g.ScanLine]; if ($g.InVerticalBlank) { $vb++ }
  if ($SleepMs -gt 0) { Start-Sleep -Milliseconds $SleepMs }
}
$top = ($lines.GetEnumerator() | Sort-Object Value -Descending | Select -First 5 | % { "$($_.Key):$($_.Value)" }) -join ' '
"SCAN DONE n=$N ms=$([int]$sw.Elapsed.TotalMilliseconds) distinct_scanlines=$($lines.Count) in_vblank=$vb errors=$err $first top=$top at $((Get-Date).ToString('HH:mm:ss.fff'))" | Add-Content $Out
