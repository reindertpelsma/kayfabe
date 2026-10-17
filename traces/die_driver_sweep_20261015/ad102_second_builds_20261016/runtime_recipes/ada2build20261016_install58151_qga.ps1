$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep';New-Item -ItemType Directory -Force $root | Out-Null
$marker=Join-Path $root 'driver58151.started'
if(Test-Path $marker) { throw 'An earlier install start exists; inspect outcome instead of replaying' }
$parts=@(Get-ChildItem (Join-Path $root 'downloads\nvidia58151.part*') | Sort-Object Name)
if($parts.Count -ne 218) { throw 'Require all218NVIDIA chunks' }
$exe=Join-Path $root 'driver58151.exe'
$output=[IO.File]::Open($exe,[IO.FileMode]::Create,[IO.FileAccess]::Write)
try {
 for($i=0;$i -lt 218;$i++) {
  if($parts[$i].Name -ne ('nvidia58151.part{0:d3}' -f $i)) { throw 'NVIDIA chunk sequence mismatch' }
  $input=[IO.File]::OpenRead($parts[$i].FullName)
  try { $input.CopyTo($output) } finally { $input.Dispose() }
 }
} finally { $output.Dispose() }
if((Get-FileHash $exe).Hash.ToLower() -ne 'cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a') { throw 'NVIDIA package hash mismatch' }
$archiver=Join-Path $root '7zr.exe'
if((Get-FileHash $archiver).Hash.ToLower() -ne '256feca8e274e5da655e2a284fabafd9f554365eb164862089dacd4e8276d282') { throw 'Public archiver pin mismatch' }
$dest=Join-Path $root 'nvidia58151';New-Item -ItemType Directory -Force $dest | Out-Null
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
 Write-Output 'NVIDIA_INSTALL_COMPLETE=1; direct actual8151/GSP binding and cold ABI580.94.02 boot required'
} finally { "INSTALL_EXIT rc=$([int](!$ok)) utc=$([DateTime]::UtcNow.ToString('o'))" | Set-Content (Join-Path $root 'driver58151.exit') }
