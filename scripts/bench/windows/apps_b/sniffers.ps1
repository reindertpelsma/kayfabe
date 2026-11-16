# The applications' own GPU diagnostics, offline: the sniffer programs that ship with the apps print the GPU they would use.
# No window is opened. Output is saved as text for the host to grade.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'sniffers'
try {
  $R.outputs = @()
  foreach ($e in @(
      @('premiere', 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\GPUSniffer.exe'),
      @('aftereffects', 'C:\Program Files\Adobe\Adobe After Effects 2027\Support Files\GPUSniffer.exe'),
      @('mediaencoder', 'C:\Program Files\Adobe\Adobe Media Encoder 2027\GPUSniffer.exe'),
      @('photoshop', 'C:\Program Files\Adobe\Adobe Photoshop 2027\sniffer.exe'))) {
    if (-not (Test-Path $e[1])) { continue }
    $f = "$D\sniffer_$($e[0]).txt"
    ((cmd.exe /c "`"$($e[1])`" 2>&1") | Out-String) | Set-Content -Encoding UTF8 $f
    $R.outputs += (File-Info $f)
  }
} catch { $R.error = $_.Exception.Message }
Finish $R
