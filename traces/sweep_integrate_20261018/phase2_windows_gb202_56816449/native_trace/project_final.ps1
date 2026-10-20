$ErrorActionPreference='Stop'
$r='C:\ProgramData\KayfabeSweepNative'
$s=Get-Content "$r\revisit.foreground.stdout" -Raw
if($s -notmatch 'FINAL_NATIVE_TRACE_EXIT rc=(\d+)') { throw 'Final controller not complete' }
$finalRc=[int]$Matches[1]
$rows=@()
foreach($n in 'seedwindow580','revisitwindow580') {
 $j=Get-Content "$r\$n\cell.json" -Raw | ConvertFrom-Json
 $cap=Get-Item "$r\$n\capture.kfgspcap"
 $collector='UNVERIFIED';if($s -match "COLLECTOR_EXIT name=$n rc=(\d+)"){$collector=[int]$Matches[1]}
 $census='UNVERIFIED';$sequence='UNVERIFIED';if($s -match "DECODE_EXIT name=$n census=(\d+) sequence=(\d+)"){$census=[int]$Matches[1];$sequence=[int]$Matches[2]}
 $rows+= [ordered]@{arm=$n;raw_bytes=$cap.Length;raw_sha256=(Get-FileHash $cap.FullName).Hash;discovery=($j.discovery | Select-Object method,layout,mailbox_reads,mailbox_status,scan_passes,scan_bytes);stats=$j.stats;capture=$j.capture;collector_exit_code=$collector;strict_census_rc=$census;sequence_rc=$sequence}
}
$g=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
$sign=Get-AuthenticodeSignature "$env:SystemRoot\System32\drivers\kfwc.sys"
$proof=[ordered]@{driver=($g | Select-Object Name,DriverVersion,ConfigManagerErrorCode,Status);firmware=((& nvidia-smi.exe -q | Select-String 'GSP Firmware Version').ToString().Trim());module_service=(Get-Service KayfabeWinCapture).Status.ToString();signed_module_sha256=(Get-FileHash "$env:SystemRoot\System32\drivers\kfwc.sys").Hash;signature=$sign.Status.ToString();signer_thumbprint=$sign.SignerCertificate.Thumbprint;source='8f9bd4e519fdd085c5b90e39542e0a3454145d75';final_controller_rc=$finalRc;complete_prefix=($s.Contains('NATIVE_COMPLETE_PREFIX_PASS=1'));raw_stays_on_rental=$true;arms=$rows}
$proof | ConvertTo-Json -Depth 10 | Set-Content "$r\final_public_receipt.json" -Encoding UTF8
$proof | ConvertTo-Json -Depth 10
