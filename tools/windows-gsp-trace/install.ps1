# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Run elevated, on the disposable Windows research target only.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('EnableTestSigning','Install','Remove')][string]$Action,
    [string]$DriverPath = "$PSScriptRoot\build\gsptrace.sys",
    [string]$SignToolPath
)
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run elevated.' }
if ($Action -eq 'EnableTestSigning') {
    & bcdedit.exe /set '{current}' testsigning on
    if ($LASTEXITCODE) { throw 'Test signing could not be enabled. Prepare the disposable VM with Secure Boot disabled.' }
    Write-Output 'Test signing configured. Reboot Windows, then run Install. No reboot was performed by this script.'
    return
}
if ($Action -eq 'Remove') {
    & sc.exe stop KayfabeGspTrace | Out-Host
    if ($LASTEXITCODE -notin @(0,1060,1062)) { throw "Cannot stop driver: $LASTEXITCODE" }
    & sc.exe delete KayfabeGspTrace | Out-Host
    if ($LASTEXITCODE -notin @(0,1060)) { throw "Cannot delete service: $LASTEXITCODE" }
    Write-Output 'Driver service removed. The test certificate and test-signing boot setting remain for explicit cleanup.'
    return
}
if (-not (Test-Path -LiteralPath $DriverPath)) { throw "Missing driver: $DriverPath" }
if (-not $SignToolPath -and (Test-Path "$PSScriptRoot\build\signing\signtool.exe")) { $SignToolPath = "$PSScriptRoot\build\signing\signtool.exe" }
if (-not $SignToolPath) {
    $tool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue | Sort-Object FullName | Select-Object -Last 1
    if ($tool) { $SignToolPath = $tool.FullName }
}
if (-not $SignToolPath -or -not (Test-Path -LiteralPath $SignToolPath)) { throw 'Provide -SignToolPath from the Microsoft Windows SDK.' }
$destination = "$env:SystemRoot\System32\drivers\kayfabe-gsptrace.sys"
if (Get-Service KayfabeGspTrace -ErrorAction SilentlyContinue) { throw 'Service already exists. Collect/drain the previous run and Remove before Install.' }
Copy-Item -LiteralPath $DriverPath -Destination $destination -Force
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=Kayfabe disposable GSP observer test' -CertStoreLocation Cert:\LocalMachine\My -HashAlgorithm SHA256
$publicCertificate = Join-Path $env:TEMP 'kayfabe-gsptrace-test.cer'
Export-Certificate -Cert $cert -FilePath $publicCertificate -Force | Out-Null
$public = [Security.Cryptography.X509Certificates.X509Certificate2]::new($publicCertificate)
try {
    foreach ($name in @('Root','TrustedPublisher')) {
        $store = [Security.Cryptography.X509Certificates.X509Store]::new($name, [Security.Cryptography.X509Certificates.StoreLocation]::LocalMachine)
        try {
            # Open(ReadWrite) creates an absent store. Import-Certificate can
            # report E_ACCESSDENIED for TrustedPublisher on a fresh Windows VM.
            $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
            $store.Add($public)
            if (-not $store.Certificates.Find([Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint, $cert.Thumbprint, $false).Count) { throw "Test certificate missing from LocalMachine\$name" }
        } finally { $store.Close() }
    }
} finally { $public.Dispose() }
& $SignToolPath sign /fd SHA256 /sha1 $cert.Thumbprint /sm /s My $destination
if ($LASTEXITCODE) { throw 'Driver signing failed.' }
& $SignToolPath verify /pa /v $destination
if ($LASTEXITCODE) { throw 'Driver signature verification failed.' }
& sc.exe create KayfabeGspTrace type= kernel start= demand binPath= $destination
if ($LASTEXITCODE) { throw 'Service creation failed.' }
& sc.exe start KayfabeGspTrace
if ($LASTEXITCODE) { throw 'Driver load failed. Check testsigning after reboot and CodeIntegrity event log.' }
Write-Output "Recorder is scanning. Test certificate thumbprint: $($cert.Thumbprint)"
Write-Output 'Start gsptrace.exe OUTPUT.kgwt SECONDS in another elevated terminal, then enable/install NVIDIA with GSP enabled.'
