#!/bin/bash
# prepc.sh : one-time guest preparation on the private overlay K.qcow2 (backing = the owner's apps disk, never written): boot on the candidate,
# push the kfperf helpers, install Heaven and the VC runtime from the KFAPPS CD, shut down. Caller: flock -o /tmp/kayfabe-fastguest.lock bash prepc.sh
G=/data/claude-candhw/gap; cd $G; P=$G/perf
export OVL=$G/disk/K.qcow2 VARS=$G/disk/K.vars.fd RAM_MB=12288 FB_MB=8192 BAR1=8589934592 CHAIN_LOCKED=1
echo "PREP_START $(date -Is)"
REV=${REV:-525d7d7a} bash $P/startk.sh PREP ""
for f in kfp_body.ps1 kfp_launch.ps1 kfp_smi.ps1; do bash $P/kfpush.sh $P/guest/$f "C:\\kfperf\\$f"; done
cat > $G/ev/stage_heaven2.ps1 <<'PS'
$cd = (Get-Volume | Where-Object { $_.FileSystemLabel -eq "KFAPPS" } | Select-Object -First 1).DriveLetter
"CD=$cd"
if (-not $cd) { Get-Volume | ForEach-Object { "VOL $($_.DriveLetter) [$($_.FileSystemLabel)] $($_.DriveType)" }; return }
$cd = "${cd}:\"
New-Item -ItemType Directory -Force C:\kfapps | Out-Null
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
"VCR " + (Test-Path C:\Windows\System32\vcruntime140_1.dll)
Get-ChildItem C:\kfapps\heaven\bin -ErrorAction SilentlyContinue | Select-Object -First 8 | ForEach-Object { $_.Name }
"HEAVEN_STAGE_DONE"
PS
EV=$G/ev bash $P/kfp.sh stage_heaven service $G/ev/stage_heaven2.ps1 900
bash $P/stopk.sh PREP > ev/stop_PREP.out 2>&1
echo "PREP_EXIT $(date -Is)"
