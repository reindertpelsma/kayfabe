# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# GPU-free source-build leg of package_open_bundle.sh --release-assets DIR.
# Run once on a Windows packaging workstation with MSVC v143, Windows SDK, dxc and nuget.exe.
# Source must be a clean checkout at the exact commit in release_software.json. No sample is run.
# Output is an offline asset tree and a per-file SHA-256 receipt. No upload or deployment.
param([Parameter(Mandatory = $true)][string]$Source,
      [Parameter(Mandatory = $true)][string]$Out, [string]$References = '', [switch]$DryRun)
$ErrorActionPreference = 'Stop'
if ($env:CI -or $env:KF_RUN_TOKEN) { throw 'Person-only packaging step; never on a rented box or CI' }
$cfg = Get-Content -Raw (Join-Path $PSScriptRoot 'release_software.json') | ConvertFrom-Json
$Source = (Resolve-Path -LiteralPath $Source).Path
$head = (& git -C $Source rev-parse HEAD)
if ($LASTEXITCODE -ne 0 -or $head -ne $cfg.directx.commit) { throw 'DirectX source commit mismatch' }
$dirty = (& git -C $Source status --porcelain --untracked-files=no)
if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'DirectX tracked source is dirty' }
if (Test-Path -LiteralPath $Out) { throw 'Output must be a new directory (no stale binaries)' }
$projects = @($cfg.directx.projects.PSObject.Properties)
foreach ($p in $projects) {
    if (-not (Test-Path -LiteralPath (Join-Path $Source $p.Value))) { throw "Missing source project $($p.Name)" }
}
$patch = Join-Path $PSScriptRoot 'directx_vrs_gate.patch'
$patchHash = (Get-FileHash -LiteralPath $patch -Algorithm SHA256).Hash.ToLowerInvariant()
if ($patchHash -ne $cfg.directx.patches.vrs_gate) { throw 'VRS gate patch hash mismatch' }
& git -C $Source apply --check $patch
if ($LASTEXITCODE -ne 0) { throw 'VRS gate patch does not apply to the pinned source' }
if ($DryRun) {
    "RELEASE_BUILD_PLAN commit=$head projects=$($projects.Count) gpu_runs=0"
    'SOURCE-SAMPLE-NEEDED sampler_feedback: no project in the pinned Microsoft tree'
    exit 0
}
$null = Get-Command msbuild.exe -ErrorAction Stop
$null = Get-Command nuget.exe -ErrorAction Stop
$Out = [IO.Path]::GetFullPath($Out)
New-Item -ItemType Directory -Path $Out | Out-Null
$originalSource = $Source
$buildSource = Join-Path ([IO.Path]::GetTempPath()) ('kf-directx-build-' + [guid]::NewGuid().ToString('N'))
& git -C $originalSource worktree add --detach $buildSource $head
if ($LASTEXITCODE -ne 0) { throw 'Could not create the private source build worktree' }
try {
$Source = $buildSource
& git -C $Source apply $patch
if ($LASTEXITCODE -ne 0) { throw 'VRS gate patch failed' }
foreach ($p in $projects) {
    $proj = Join-Path $Source $p.Value
    $dir = Split-Path $proj
    $pkgConfig = Join-Path $dir 'packages.config'
    if ($p.Name -eq 'dxr') { $pkgDir = Join-Path $Source 'Packages' }
    elseif ($p.Name -eq 'mesh') { $pkgDir = Join-Path (Split-Path $dir) 'packages' }
    else { $pkgDir = Join-Path $dir 'packages' }
    # packages.config pins each dependency version at the pinned source revision.
    & nuget.exe install $pkgConfig -OutputDirectory $pkgDir -NonInteractive -Source 'https://api.nuget.org/v3/index.json'
    if ($LASTEXITCODE -ne 0) { throw "NuGet restore failed: $($p.Name)" }
    $dest = Join-Path $Out ('directx\' + $p.Name)
    New-Item -ItemType Directory -Path $dest | Out-Null
    Push-Location $dir
    try {
        & msbuild.exe $proj /m:2 /t:Rebuild /p:Configuration=Release /p:Platform=x64 /p:PlatformToolset=v143 /p:DefaultPlatformToolset=v143 "/p:OutDir=$dest\" "/p:IntDir=$dest\obj\" "/p:SolutionDir=$dir\"
        if ($LASTEXITCODE -ne 0) { throw "MSBuild failed: $($p.Name)" }
    } finally { Pop-Location }
    # Include runtime shader/data assets as well as the compiler's output/dependency DLLs.
    Get-ChildItem -LiteralPath $dir -File | Where-Object { $_.Extension -in @('.hlsl', '.hlsli', '.dds', '.bin') } | Copy-Item -Destination $dest
    Remove-Item -LiteralPath (Join-Path $dest 'obj') -Recurse -Force -ErrorAction SilentlyContinue
    $licenses = Join-Path $Out ('licenses\' + $p.Name)
    New-Item -ItemType Directory -Path $licenses -Force | Out-Null
    Copy-Item -LiteralPath $pkgConfig -Destination $licenses
    Get-ChildItem -LiteralPath $pkgDir -Directory | ForEach-Object {
        $pd = Join-Path $licenses $_.Name
        New-Item -ItemType Directory -Path $pd -Force | Out-Null
        Get-ChildItem -LiteralPath $_.FullName -File | Where-Object { $_.Name -match 'license|notice|copying|\.nuspec$' } | Copy-Item -Destination $pd
    }
}
$assets = Join-Path $Out 'directx\Assets'
New-Item -ItemType Directory -Path $assets -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $Source 'Samples\Desktop\D3D12MeshShaders\src\Assets\Dragon_LOD0.bin') -Destination $assets
Copy-Item -LiteralPath (Join-Path $Source 'LICENSE') -Destination (Join-Path $Out 'directx\LICENSE.txt')
Copy-Item -LiteralPath $patch -Destination (Join-Path $Out 'directx\GATE_VRS.patch')
if ($References) {
    $referenceNames = @($cfg.apps.PSObject.Properties | ForEach-Object { $_.Value.assets } | Where-Object { $_ -like 'references/*' } | Sort-Object -Unique)
    New-Item -ItemType Directory -Path (Join-Path $Out 'references') -Force | Out-Null
    foreach ($rel in $referenceNames) {
        $file = Join-Path $References (Split-Path $rel -Leaf)
        if (-not (Test-Path -LiteralPath $file) -or (Get-Item -LiteralPath $file).Length -eq 0) { throw "Missing/empty reference: $rel" }
        Copy-Item -LiteralPath $file -Destination (Join-Path $Out $rel)
    }
}
foreach ($rel in $cfg.directx.required_files) {
    $file = Join-Path $Out $rel
    if (-not (Test-Path -LiteralPath $file) -or (Get-Item -LiteralPath $file).Length -eq 0) { throw "Missing/empty built asset: $rel" }
}
$files = [ordered]@{}
Get-ChildItem -LiteralPath $Out -Recurse -File | Sort-Object FullName | ForEach-Object {
    $rel = $_.FullName.Substring($Out.Length).TrimStart('\', '/').Replace('\', '/')
    $files[$rel] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}
$receipt = [ordered]@{schema=1; source_repository=$cfg.directx.repository; source_commit=$head; built_projects=$cfg.directx.projects; patches=$cfg.directx.patches; files=$files}
$receipt | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $Out 'receipt.json') -Encoding UTF8
"RELEASE_ASSETS_BUILT files=$($files.Count) gpu_runs=0; unverified until hardware"
'SOURCE-SAMPLE-NEEDED sampler_feedback; references must be collected separately on bare metal'
} finally {
    & git -C $originalSource worktree remove --force $buildSource
    if ($LASTEXITCODE -ne 0) { Write-Warning "Could not remove this build's worktree: $buildSource" }
}
