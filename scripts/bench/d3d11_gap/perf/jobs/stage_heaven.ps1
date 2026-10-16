# job: install Heaven and the VC runtime from D: (KFAPPS)
$cd = "D:\"
if (-not (Test-Path C:\kfapps\heaven\bin\Heaven.exe)) {
  $h = Get-ChildItem (Join-Path $cd "pkg\heaven") | Select-Object -First 1
  "HEAVEN installer=$($h.Name)"
  $p = Start-Process $h.FullName -ArgumentList @("/VERYSILENT","/SUPPRESSMSGBOXES","/NORESTART","/SP-","/DIR=C:\kfapps\heaven") -Wait -PassThru
  "HEAVEN rc=$($p.ExitCode) exe=$(Test-Path C:\kfapps\heaven\bin\Heaven.exe)"
}
if (-not (Test-Path C:\Windows\System32\vcruntime140_1.dll)) {
  $v = Get-ChildItem (Join-Path $cd "pkg\vc_redist") | Select-Object -First 1
  $p = Start-Process $v.FullName -ArgumentList @("/install","/quiet","/norestart") -Wait -PassThru; "VCREDIST rc=$($p.ExitCode)"
}
Get-ChildItem C:\kfapps\heaven -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }
Get-ChildItem C:\kfapps\heaven\bin -ErrorAction SilentlyContinue | Select-Object -First 20 | ForEach-Object { $_.Name }
"HEAVEN_STAGE_DONE"
