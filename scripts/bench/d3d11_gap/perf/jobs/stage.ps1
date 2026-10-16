# job: stage the benchmark apps from the KFAPPS CD into C:\kfapps (service mode). Idempotent.
$cd = (Get-Volume | Where-Object { $_.FileSystemLabel -eq "KFAPPS" } | Select-Object -First 1).DriveLetter
"CD=$cd"
if (-not $cd) { Get-Volume | ForEach-Object { "VOL $($_.DriveLetter) [$($_.FileSystemLabel)] $($_.DriveType)" }; return }
$cd = "${cd}:\"
New-Item -ItemType Directory -Force C:\kfapps | Out-Null
function Unz($pkg, $zip, $dest) {
  if (Test-Path "C:\kfapps\$dest\.ok") { "SKIP $dest"; return }
  New-Item -ItemType Directory -Force "C:\kfapps\$dest" | Out-Null
  & tar.exe -xf (Join-Path $cd "pkg\$pkg\$zip") -C "C:\kfapps\$dest" 2>&1 | Out-Null
  "UNZIP $dest rc=$LASTEXITCODE"; if ($LASTEXITCODE -eq 0) { "" | Set-Content "C:\kfapps\$dest\.ok" }
}
Get-ChildItem (Join-Path $cd "pkg") | ForEach-Object { "PKG $($_.Name): " + ((Get-ChildItem $_.FullName | ForEach-Object { $_.Name }) -join " ") } | Select-String -Pattern "vkpeak|clpeak|furmark|gravity|vc_redist|heaven" -CaseSensitive:$false
$z = Get-ChildItem (Join-Path $cd "pkg\vkpeak") -Filter *.zip | Select-Object -First 1; Unz "vkpeak" $z.Name "vkpeak"
$z = Get-ChildItem (Join-Path $cd "pkg\furmark2") -Filter *.zip | Select-Object -First 1; Unz "furmark2" $z.Name "furmark2"
$z = Get-ChildItem (Join-Path $cd "pkg\clpeak_ocl") -Filter *.zip | Select-Object -First 1; Unz "clpeak_ocl" $z.Name "clpeak_ocl"
$z = Get-ChildItem (Join-Path $cd "pkg\clpeak_cuda") -Filter *.zip | Select-Object -First 1; Unz "clpeak_cuda" $z.Name "clpeak_cuda"
if (-not (Test-Path "C:\kfapps\gravitymark\.ok")) {
  New-Item -ItemType Directory -Force C:\kfapps\gravitymark | Out-Null
  $m = Get-ChildItem (Join-Path $cd "pkg\gravitymark") -Filter *.msi | Select-Object -First 1
  $p = Start-Process msiexec.exe -ArgumentList @("/a", "`"$($m.FullName)`"", "/qn", "TARGETDIR=`"C:\kfapps\gravitymark`"") -Wait -PassThru
  "MSI rc=$($p.ExitCode)"; if ($p.ExitCode -eq 0) { "" | Set-Content C:\kfapps\gravitymark\.ok }
}
New-Item -ItemType Directory -Force C:\kfapps\tools | Out-Null; Copy-Item -Force (Join-Path $cd "tools\*.exe") C:\kfapps\tools; Get-ChildItem C:\kfapps\tools | ForEach-Object { "TOOL $($_.Name) $($_.Length)" }
"EXES:"; Get-ChildItem C:\kfapps -Recurse -Include vkpeak.exe,furmark.exe,clpeak.exe,GravityMark.exe | ForEach-Object { $_.FullName }
"BLENDER:"; Test-Path "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"; Get-ChildItem C:\kf\apps\blender -ErrorAction SilentlyContinue | Select-Object -First 15 | ForEach-Object { $_.Name }
"VCRT:"; Test-Path C:\Windows\System32\vcruntime140.dll
"STAGE_DONE"
