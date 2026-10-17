$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep';New-Item -ItemType Directory -Force $root | Out-Null
$marker=Join-Path $root 'driver58088.started'
if(Test-Path $marker) { throw 'An earlier install start exists; inspect outcome instead of replaying' }
$volumes=@(Get-PSDrive -PSProvider FileSystem | Where-Object { Test-Path (Join-Path $_.Root 'driver58088.exe') })
if($volumes.Count -ne 1) { throw 'Require exactly one read-only NVIDIA payload volume' }
$exe=Join-Path $root 'driver58088.exe'
Copy-Item (Join-Path $volumes[0].Root 'driver58088.exe') $exe -Force
foreach($name in '7zr.exe','cup2.exe','cup3.exe','cup8.exe','kf_dxprobe.exe') {
 Copy-Item (Join-Path $volumes[0].Root $name) (Join-Path $root $name) -Force
}
if((Get-FileHash $exe).Hash.ToLower() -ne '90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9') { throw 'NVIDIA package hash mismatch' }
$archiver=Join-Path $root '7zr.exe'
if((Get-FileHash $archiver).Hash.ToLower() -ne '256feca8e274e5da655e2a284fabafd9f554365eb164862089dacd4e8276d282') { throw 'Public archiver pin mismatch' }
$dest=Join-Path $root 'nvidia58088';New-Item -ItemType Directory -Force $dest | Out-Null
& $archiver x -y "-o$dest" $exe | Out-Null
if($LASTEXITCODE -ne 0) { throw 'NVIDIA extraction failed' }
$setup=Join-Path $dest 'setup.exe'
foreach($file in @($exe,$setup)) {
 $sig=Get-AuthenticodeSignature $file
 Write-Output "NVIDIA_SIGNATURE status=$($sig.Status) signer=$($sig.SignerCertificate.Subject)"
 if($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'NVIDIA') { throw 'NVIDIA signature verification failed' }
}
[DateTime]::UtcNow.ToString('o') | Set-Content $marker
$ok=$false
try {
 $p=Start-Process $setup -ArgumentList @('-s','-n','Display.Driver') -PassThru
 $null=$p.Handle
 Write-Output "NVIDIA_INSTALL_PID=$($p.Id)"
 if(!$p.WaitForExit(360000)) { throw 'Installer still active; inspect before any retry' }
 $p.Refresh();Write-Output "NVIDIA_INSTALL_EXIT code=$($p.ExitCode)"
 if($p.ExitCode -notin @(0,1)) { throw 'NVIDIA installation failed' }
 $ok=$true
 Write-Output 'NVIDIA_INSTALL_COMPLETE=1; direct actual8088/GSP binding and cold ABI580.65.06 boot required'
} finally { "INSTALL_EXIT rc=$([int](!$ok)) utc=$([DateTime]::UtcNow.ToString('o'))" | Set-Content (Join-Path $root 'driver58088.exit') }
