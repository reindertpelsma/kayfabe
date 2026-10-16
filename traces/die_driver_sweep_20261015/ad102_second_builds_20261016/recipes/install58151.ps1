$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeSweep';New-Item -ItemType Directory -Force $root | Out-Null
$marker=Join-Path $root 'driver58151.started'
if(Test-Path $marker) { throw 'An earlier install start exists; inspect outcome instead of replaying' }
$cd=@([IO.DriveInfo]::GetDrives() | Where-Object { $_.DriveType -eq [IO.DriveType]::CDRom -and [IO.File]::Exists((Join-Path $_.RootDirectory.FullName 'driver58151.exe')) })
if($cd.Count -ne 1) { throw 'One pinned NVIDIA payload CD required' }
$exe=Join-Path $cd[0].RootDirectory.FullName 'driver58151.exe'
if((Get-FileHash $exe).Hash.ToLower() -ne 'cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a') { throw 'NVIDIA package hash mismatch' }
$setup=Join-Path $cd[0].RootDirectory.FullName 'nvidia58151\setup.exe'
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
