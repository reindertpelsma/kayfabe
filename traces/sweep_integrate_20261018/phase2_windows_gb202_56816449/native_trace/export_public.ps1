$ErrorActionPreference='Stop'
$r='C:\ProgramData\KayfabeSweepNative';$p="$r\revisitwindow580"
$j=Get-Content "$r\final_public_receipt.json" -Raw | ConvertFrom-Json
if(!$j.complete_prefix -or $j.final_controller_rc -ne 0 -or $j.module_service -ne 'Stopped') {throw 'Complete-prefix/stopped-module gate failed'}
& "$r\build\kf-census.exe" scrub --ogkm-tag 580.65.06 --no-default-allow --allow 0x20800ab2 --out "$p\sample-only.kfgspcap" "$p\capture.kfgspcap" | Out-File "$p\scrub-summary.txt"
if($LASTEXITCODE) {throw 'Sample-only scrub failed'}
& "$r\build\kf-census.exe" --ogkm-tag 580.65.06 --bodies --out "$p\sample-only" "$p\sample-only.kfgspcap" | Out-File "$p\sample-census-summary.txt"
if($LASTEXITCODE -notin @(0,1)) {throw 'Scrubbed body census failed'}
# The only retained body is public NV2080_CTRL_GPU_GET_CCU_SAMPLE_INFO_PARAMS:
# exactly one NvU32 ccuSampleSize, no identity/address/opaque field.
Get-Content "$p\sample-only.bodies.tsv"
