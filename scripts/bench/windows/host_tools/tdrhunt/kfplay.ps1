# kfplay.ps1 TAG : run in the INTERACTIVE session (schtasks /it). Guest-side playback evidence:
#  (1) 5 captures of the player's central column, 3 s apart, pixel-diff counts (sampled every 4th px)
#  (2) GPU Engine utilisation by engine type for msedge processes (VideoDecode = NVDEC)
#  (3) full-screen PNGs kept in C:\kf for recovery from the disk
param([string]$Tag = "a")
$out = "C:\kf\play-$Tag.txt"
"start $(Get-Date -Format o)" | Out-File $out
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
$scr = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
"screen $($scr.Width)x$($scr.Height)" | Out-File $out -Append
function Cap($x, $y, $w, $h) {
  $b = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($b)
  $g.CopyFromScreen($x, $y, 0, 0, $b.Size)
  $g.Dispose()
  return $b
}
function PixDiff($a, $b) {
  $n = 0; $tot = 0
  for ($yy = 0; $yy -lt $a.Height; $yy += 4) {
    for ($xx = 0; $xx -lt $a.Width; $xx += 4) {
      $tot++
      if ($a.GetPixel($xx, $yy).ToArgb() -ne $b.GetPixel($xx, $yy).ToArgb()) { $n++ }
    }
  }
  return "$n/$tot"
}
$cx = [int]($scr.Width / 2) - 200; $cy = 200; $cw = 400; $ch = 600
$prev = $null
for ($i = 0; $i -lt 5; $i++) {
  $full = Cap 0 0 $scr.Width $scr.Height
  $full.Save("C:\kf\play-$Tag-$i.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $crop = $full.Clone((New-Object System.Drawing.Rectangle $cx, $cy, $cw, $ch), $full.PixelFormat)
  $full.Dispose()
  if ($prev) { "crop diff $($i-1)->${i}: $(PixDiff $prev $crop)" | Out-File $out -Append }
  if ($prev) { $prev.Dispose() }
  $prev = $crop
  Start-Sleep -Seconds 3
}
$ps = @{}
Get-Process msedge -ErrorAction SilentlyContinue | ForEach-Object { $ps[[string]$_.Id] = "msedge" }
$agg = @{}
try {
  $s = Get-Counter '\GPU Engine(*)\Utilization Percentage' -SampleInterval 1 -MaxSamples 4
  foreach ($smp in $s) {
    foreach ($c in $smp.CounterSamples) {
      if ($c.InstanceName -match 'pid_(\d+)_.*engtype_(.+)$') {
        $pid2 = $Matches[1]; $eng = $Matches[2]
        $nm = if ($ps.ContainsKey($pid2)) { "msedge" } else { "other" }
        $k = "$nm/$eng"
        if (-not $agg.ContainsKey($k)) { $agg[$k] = @(0.0, 0.0) }
        $agg[$k][0] += $c.CookedValue
        if ($c.CookedValue -gt $agg[$k][1]) { $agg[$k][1] = $c.CookedValue }
      }
    }
  }
  foreach ($k in ($agg.Keys | Sort-Object)) { "engine $k sum=$([math]::Round($agg[$k][0],2)) max=$([math]::Round($agg[$k][1],2))" | Out-File $out -Append }
} catch { "counter error: $_" | Out-File $out -Append }
"done $(Get-Date -Format o)" | Out-File $out -Append
