# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Run in an ELEVATED PowerShell, on a Windows machine we own and may reboot (a rented Windows
# VM or the home test guest). This installs a TEST-SIGNED measurement driver; it is not for
# machines that carry anything we do not want to lose.
#
#   .\install.ps1 -Action Preflight                  read-only checks, changes nothing
#   .\install.ps1 -Action EnableTestSigning          bcdedit testsigning on (then reboot yourself)
#   .\install.ps1 -Action Install  [-Build DIR]      sign with a throwaway cert, create service, start
#   .\install.ps1 -Action Start | Stop | Status
#   .\install.ps1 -Action Remove                     stop + delete the service (cert and bcd stay)
#   .\install.ps1 -Action Cleanup -Thumbprint HEX    remove the throwaway certificate
#
# UNTESTED: this script has been written on Linux and parsed by nothing. Run -Action Preflight
# first and read its output before anything else. See docs/design/V3_WINDOWS_CAPTURE_MODULE.md.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Preflight','EnableTestSigning','Install','Start','Stop','Status','Remove','Cleanup')][string]$Action,
    [string]$Build = "$PSScriptRoot\..\build-out",
    [string]$SignToolPath,
    [string]$Thumbprint
)
$ErrorActionPreference = 'Stop'
$service = 'KayfabeWinCapture'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run elevated.' }

function Show($name, $value, $note) { '{0,-28} {1,-14} {2}' -f $name, $value, $note }

if ($Action -eq 'Preflight') {
    # Everything here only READS. The questions are the ones that decide whether the rental can run the module at all.
    $os = Get-CimInstance Win32_OperatingSystem
    Show 'windows build' $os.BuildNumber $os.Caption
    try { $sb = Confirm-SecureBootUEFI; Show 'secure boot' $(if ($sb) {'ON  (blocks test signing)'} else {'off'}) '' } catch { Show 'secure boot' 'n/a' 'legacy BIOS or not supported (fine)' }
    $bcd = (& bcdedit.exe /enum '{current}') -join "`n"
    Show 'testsigning' $(if ($bcd -match 'testsigning\s+Yes') {'on'} else {'off'}) 'bcdedit /set {current} testsigning on needs Secure Boot off'
    try {
        $dg = Get-CimInstance -ClassName Win32_DeviceGuard -Namespace root\Microsoft\Windows\DeviceGuard
        Show 'VBS / HVCI' ("{0} / {1}" -f ($dg.VirtualizationBasedSecurityStatus), ($dg.SecurityServicesRunning -join ',')) '2 running = VBS on; 2 in services = HVCI (test signing still loads, see doc)'
    } catch { Show 'VBS / HVCI' 'unknown' $_.Exception.Message }
    $gpu = Get-CimInstance Win32_VideoController | Where-Object { $_.PNPDeviceID -like 'PCI\VEN_10DE*' }
    foreach ($g in $gpu) {
        # PCI\VEN_10DE&DEV_xxxx only; the subsystem part of the id is deliberately not printed.
        $dev = if ($g.PNPDeviceID -match 'VEN_10DE&DEV_([0-9A-F]{4})') { $Matches[1] } else { '?' }
        Show 'nvidia gpu' "DEV_$dev" ("driver {0}, status {1}" -f $g.DriverVersion, $g.Status)
    }
    if (-not $gpu) { Show 'nvidia gpu' 'NONE' 'no NVIDIA display adapter visible' }
    Show 'gsp' '(see nvidia-smi -q)' 'GSP Firmware Version must not be N/A for queues to exist'
    $free = (Get-PSDrive -Name C).Free / 1MB
    Show 'free disk C: (MiB)' ([int]$free) 'a capture of a boot is tens of MiB'
    $mem = [int]($os.TotalVisibleMemorySize / 1024)
    Show 'RAM (MiB)' $mem 'the RAM scan fallback reads all of it once per pass'
    return
}
if ($Action -eq 'EnableTestSigning') {
    & bcdedit.exe /set '{current}' testsigning on
    if ($LASTEXITCODE) { throw 'Test signing could not be enabled (Secure Boot on?). Nothing was changed.' }
    Write-Output 'testsigning is on for the NEXT boot. Reboot yourself, then run -Action Install.'
    return
}
if ($Action -in 'Status') { & sc.exe query $service; return }
if ($Action -eq 'Start') { & sc.exe start $service; return }
if ($Action -eq 'Stop') { & sc.exe stop $service; return }
if ($Action -eq 'Remove') {
    & sc.exe stop $service | Out-Host
    if ($LASTEXITCODE -notin @(0,1060,1062)) { throw "Cannot stop the service: $LASTEXITCODE" }
    & sc.exe delete $service | Out-Host
    if ($LASTEXITCODE -notin @(0,1060)) { throw "Cannot delete the service: $LASTEXITCODE" }
    Write-Output 'Service removed. The test certificate and the testsigning boot option remain; use -Action Cleanup and bcdedit /set {current} testsigning off.'
    return
}
if ($Action -eq 'Cleanup') {
    if (-not $Thumbprint) { throw 'Give -Thumbprint (printed by Install).' }
    foreach ($store in 'My','Root','TrustedPublisher') {
        Get-ChildItem "Cert:\LocalMachine\$store" | Where-Object { $_.Thumbprint -eq $Thumbprint } | Remove-Item
    }
    Write-Output 'Certificate removed. To leave test mode: bcdedit /set {current} testsigning off, then reboot.'
    return
}

# ---- Install -----------------------------------------------------------------------------------
$sys = Join-Path $Build 'kfwc.sys'
if (-not (Test-Path -LiteralPath $sys)) { throw "Missing $sys (build with build/build-linux.sh and copy the output here)." }
if (-not (Test-Path -LiteralPath (Join-Path $Build 'SHA256SUMS'))) { throw 'Missing SHA256SUMS next to the build output.' }
$expected = (Get-Content (Join-Path $Build 'SHA256SUMS') | Where-Object { $_ -match 'kfwc\.sys' }) -replace '\s.*',''
if ((Get-FileHash -Algorithm SHA256 $sys).Hash.ToLower() -ne $expected.ToLower()) { throw 'kfwc.sys does not match SHA256SUMS.' }
if (((& bcdedit.exe /enum '{current}') -join "`n") -notmatch 'testsigning\s+Yes') { throw 'testsigning is not on. Run -Action EnableTestSigning, reboot, then retry.' }
if (-not $SignToolPath) {
    $tool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue | Sort-Object FullName | Select-Object -Last 1
    if ($tool) { $SignToolPath = $tool.FullName }
}
if (-not $SignToolPath -or -not (Test-Path -LiteralPath $SignToolPath)) { throw 'Provide -SignToolPath (signtool.exe from the Windows SDK; the build can also ship one).' }
if (Get-Service $service -ErrorAction SilentlyContinue) { throw 'Service already exists. -Action Remove first.' }
$dest = "$env:SystemRoot\System32\drivers\kfwc.sys"
Copy-Item -LiteralPath $sys -Destination $dest -Force
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=Kayfabe disposable capture module test' -CertStoreLocation Cert:\LocalMachine\My -HashAlgorithm SHA256
$cer = Join-Path $env:TEMP 'kfwc-test.cer'
Export-Certificate -Cert $cert -FilePath $cer -Force | Out-Null
$public = [Security.Cryptography.X509Certificates.X509Certificate2]::new($cer)
try {
    foreach ($name in 'Root','TrustedPublisher') {
        # Open(ReadWrite) creates an absent store; Import-Certificate failed with E_ACCESSDENIED on a fresh image.
        $st = [Security.Cryptography.X509Certificates.X509Store]::new($name, [Security.Cryptography.X509Certificates.StoreLocation]::LocalMachine)
        try { $st.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite); $st.Add($public) } finally { $st.Close() }
    }
} finally { $public.Dispose() }
& $SignToolPath sign /fd SHA256 /sha1 $cert.Thumbprint /sm /s My $dest
if ($LASTEXITCODE) { throw 'Signing failed.' }
& $SignToolPath verify /pa /v $dest
if ($LASTEXITCODE) { throw 'Signature verification failed.' }
& sc.exe create $service type= kernel start= demand binPath= $dest
if ($LASTEXITCODE) { throw 'sc create failed.' }
& sc.exe start $service
if ($LASTEXITCODE) { throw 'Driver load failed (CodeIntegrity event log; Secure Boot; HVCI).' }
Write-Output "Loaded. Certificate thumbprint (keep for -Action Cleanup): $($cert.Thumbprint)"
Write-Output 'Next: kfwc_collect.exe C:\traces\run1 --seconds 600   (the folder must exist and be empty)'
