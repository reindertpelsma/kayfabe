# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Uses the runner's installed Microsoft compiler/linker; no signing or credentials.
[CmdletBinding()]
param(
    [string]$OutputDirectory = "$PSScriptRoot\build-msvc",
    [string]$CacheDirectory = (Join-Path ([IO.Path]::GetTempPath()) 'kayfabe-gsp-msvc-kit')
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$PSNativeCommandUseErrorActionPreference = $false
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$CacheDirectory = [IO.Path]::GetFullPath($CacheDirectory)
New-Item -ItemType Directory -Force $OutputDirectory,$CacheDirectory | Out-Null
$log = Join-Path $OutputDirectory 'build.log'
Set-Content -LiteralPath $log -Value 'Unsigned MSVC kernel-driver build' -Encoding UTF8
Set-Location $PSScriptRoot

function Invoke-BuildTool([string]$Tool, [string[]]$ToolArguments) {
    [ordered]@{tool=$Tool;arguments=$ToolArguments} | ConvertTo-Json -Compress | Add-Content -LiteralPath $log
    & $Tool @ToolArguments 2>&1 | Tee-Object -FilePath $log -Append | Write-Host
    if ($LASTEXITCODE) { throw "$Tool failed with exit code $LASTEXITCODE" }
}
function Get-PinnedKit([string]$Package, [string]$Key, [string]$ExpectedHash) {
    $archive = Join-Path $CacheDirectory "$Key.zip"
    $destination = Join-Path $CacheDirectory $Key
    if (-not (Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $ExpectedHash) {
        Invoke-WebRequest -Uri "https://api.nuget.org/v3-flatcontainer/$Package/10.0.28000.2526/$Package.10.0.28000.2526.nupkg" -OutFile $archive
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $ExpectedHash) { throw "SHA256 mismatch: $Package" }
    if (-not (Test-Path "$destination\extracted.ok")) {
        Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
        Set-Content "$destination\extracted.ok" $ExpectedHash -Encoding ASCII
    }
    return $destination
}

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'An installed Microsoft Visual C++ x64 toolchain is required.' }
$visualStudio = [string](& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
if ($LASTEXITCODE -or -not $visualStudio) { throw 'Visual C++ tools not found.' }
$devCommand = Join-Path $visualStudio 'Common7\Tools\VsDevCmd.bat'
# Capture rather than print the environment; unrelated runner variables may be private.
$environmentLines = & $env:ComSpec /d /c ('"'+$devCommand+'" -arch=amd64 -host_arch=amd64 >nul && set')
if ($LASTEXITCODE) { throw 'VsDevCmd failed.' }
foreach ($line in $environmentLines) {
    $equals = $line.IndexOf('=')
    if ($equals -gt 0) { [Environment]::SetEnvironmentVariable($line.Substring(0,$equals),$line.Substring($equals+1),'Process') }
}
$cl = (Get-Command cl.exe -ErrorAction Stop).Source
$link = (Get-Command link.exe -ErrorAction Stop).Source
$wdkHash = '63c939fb5a79295bf40e941db592681272219b04edff095fe2f3d123e5579a90'
$sdkHash = 'be1b419491607eae6f7c57844ebab39face9643c51e2af1d9176a3ba0d0b23fc'
$wdk = Get-PinnedKit 'microsoft.windows.wdk.x64' 'wdk' $wdkHash
$sdk = Get-PinnedKit 'microsoft.windows.sdk.cpp' 'sdk' $sdkHash
$kit = '10.0.28000.0'
$compile = @('/nologo','/c','/kernel','/std:c11','/W4','/WX','/O2','/GS','/D_AMD64_','/DAMD64','/D_WIN64',
    '/D_WIN32_WINNT=0x0A00','/DNTDDI_VERSION=0x0A000008',
    "/I$wdk\c\Include\$kit\km", "/I$wdk\c\Include\$kit\km\crt",
    "/I$sdk\c\Include\$kit\shared", "/I$sdk\c\Include\$kit\ucrt")
$linkOptions = @('/nologo','/driver','/subsystem:native,10.0','/entry:GsDriverEntry','/machine:x64',
    '/nodefaultlib','/dynamicbase','/nxcompat','/integritycheck','/release','/Brepro',
    "/libpath:$wdk\c\Lib\$kit\km\x64")
# MSVC derives the OS version from /subsystem; /osversion is LLD-specific.
$revision = [string](& git rev-parse HEAD)
if ($LASTEXITCODE) { throw 'Cannot identify source revision.' }
$metadata = [ordered]@{schema='kayfabe-gsp-msvc-build/1';source_revision=$revision.Trim();
    timestamp_utc=(Get-Date).ToUniversalTime().ToString('o');compiler=$cl;linker=$link;
    compiler_version=(Get-Item -LiteralPath $cl).VersionInfo.FileVersion;
    linker_version=(Get-Item -LiteralPath $link).VersionInfo.FileVersion;
    runner_image=$env:ImageOS;runner_image_version=$env:ImageVersion;
    sdk_nuget_version='10.0.28000.2526';wdk_nuget_version='10.0.28000.2526';
    sdk_sha256=$sdkHash;wdk_sha256=$wdkHash;signing_performed=$false;completed=$false}
$metadata | ConvertTo-Json -Depth 6 | Set-Content "$OutputDirectory\build-info.json" -Encoding UTF8

try {
    Invoke-BuildTool $cl ($compile + @("/Fo$OutputDirectory\queue.obj",'queue.c'))
    foreach ($variant in @('gsptrace','gsptrace-diag')) {
        $options = $compile + @("/Fo$OutputDirectory\$variant.obj",'gsptrace.c')
        if ($variant -eq 'gsptrace-diag') { $options += '/DKFGT_INIT_DIAGNOSTICS' }
        Invoke-BuildTool $cl $options
        Invoke-BuildTool $link ($linkOptions + @("/out:$OutputDirectory\$variant.sys","$OutputDirectory\$variant.obj","$OutputDirectory\queue.obj",'ntoskrnl.lib','hal.lib','wdmsec.lib','BufferOverflowK.lib'))
    }
    Invoke-BuildTool $cl ($compile + @("/Fo$OutputDirectory\load_probe.obj",'tests\load_probe.c'))
    Invoke-BuildTool $link ($linkOptions + @("/out:$OutputDirectory\load-probe.sys","$OutputDirectory\load_probe.obj",'ntoskrnl.lib','BufferOverflowK.lib'))
    foreach ($source in @('collect.c','tests\windows_api_test.c')) {
        $name = if ($source -eq 'collect.c') { 'gsptrace' } else { 'windows_api_test' }
        Invoke-BuildTool $cl @('/nologo','/std:c11','/W4','/WX','/O2','/D_CRT_SECURE_NO_WARNINGS',"/Fe$OutputDirectory\$name.exe","/Fo$OutputDirectory\$name-user.obj",$source,'advapi32.lib')
    }
    $metadata.completed = $true
} finally {
    $metadata['artifacts'] = @(Get-ChildItem -LiteralPath $OutputDirectory -File | Where-Object Extension -in @('.sys','.exe') | ForEach-Object {
        [ordered]@{name=$_.Name;bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
    })
    $metadata | ConvertTo-Json -Depth 6 | Set-Content "$OutputDirectory\build-info.json" -Encoding UTF8
}
