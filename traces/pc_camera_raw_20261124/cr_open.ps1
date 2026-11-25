# cr_open.ps1 -File path : start Photoshop with one file, working directory = the Photoshop folder; wait for a window
param([string]$File)
. C:\kfhw\drv\uia_lib.ps1
$exe = "C:\Program Files\Adobe\Adobe Photoshop 2027\Photoshop.exe"
"START " + (Get-Date).ToUniversalTime().ToString("s") + "Z"
Start-Process -FilePath $exe -ArgumentList ('"' + $File + '"') -WorkingDirectory (Split-Path $exe)
$t0 = Get-Date; $found = $false
while (((Get-Date) - $t0).TotalSeconds -lt 600) {
  Start-Sleep 5
  foreach ($w in (Windows-Of @("Photoshop"))) { if ($w.Current.Name) { $found = $true } }
  if ($found) { Start-Sleep 20; break }
}
"window found: $found after " + [int]((Get-Date) - $t0).TotalSeconds + " s"
foreach ($w in (Windows-Of @("Photoshop"))) { "WIN | " + (Scrub $w.Current.Name) + " | " + $w.Current.ClassName }
