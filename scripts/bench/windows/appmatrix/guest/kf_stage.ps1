# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_stage.ps1 -Pkg ID -- make one package of the app disk usable: extract/copy/install it under C:\kfapps
# (or, for a pre-extracted tree package, just note that it runs in place from the disk). The package table
# is C:\kf\stage.json, pushed by the driver from manifest.json. Idempotent (marker C:\kfapps\.staged\ID.json).
# Output: one `KFSTAGE {json}` line. Exit 0 ok, 4 failed.
param([Parameter(Mandatory = $true)][string]$Pkg)
$ErrorActionPreference = 'Continue'
. "$PSScriptRoot\kf_common.ps1"
Initialize-KfDirs
$t0 = Get-Date
$res = @{ pkg = $Pkg; ok = $false; mode = ''; secs = 0; detail = '' }
function Done($ok, $detail) {
    $res.ok = $ok; $res.detail = $detail; $res.secs = [int]((Get-Date) - $t0).TotalSeconds
    if ($ok) { Write-JsonAtomic $res "C:\kfapps\.staged\$Pkg.json" }
    Write-Output ('KFSTAGE ' + (ConvertTo-Json -InputObject $res -Depth 4 -Compress))
    if ($ok) { exit 0 } else { exit 4 }
}
$marker = "C:\kfapps\.staged\$Pkg.json"
if (Test-Path $marker) { $m = Read-Json $marker; if ($m.ok) { $res.mode = 'cached'; $res.ok = $true; $res.detail = 'already staged'; Write-Output ('KFSTAGE ' + (ConvertTo-Json -InputObject $res -Compress)); exit 0 } }
$tab = Read-Json 'C:\kf\stage.json'
if (-not $tab) { Done $false 'C:\kf\stage.json missing' }
$def = $tab.$Pkg
if (-not $def) { Done $false "package $Pkg not in stage.json" }
$cd = (Get-Content -Raw 'C:\kf\cd.txt').Trim()
if (-not $cd -or -not (Test-Path $cd)) { Done $false 'app disk not mounted' }
$res.mode = $def.mode
$src = Join-Path $cd ('pkg\' + $Pkg)
$dest = Join-Path 'C:\kfapps' $def.dest

function Flatten-SingleTop([string]$d) {
    $items = @(Get-ChildItem -Force -Path $d)
    if ($items.Count -eq 1 -and $items[0].PSIsContainer) {
        $top = $items[0].FullName
        Get-ChildItem -Force -Path $top | Move-Item -Destination $d -Force
        Remove-Item -Force -Recurse $top
    }
}

switch ($def.mode) {
    'tree' { Done $true "runs in place from $cd" }
    'unzip' {
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        foreach ($f in $def.files) {
            $zip = Join-Path $src $f
            & tar.exe -xf $zip -C $dest 2>&1 | Out-Null
            if ($LASTEXITCODE -ne 0) { Done $false "tar -xf $f failed rc=$LASTEXITCODE" }
        }
        if ($def.strip_top) { Flatten-SingleTop $dest }
        Done $true "extracted to $dest"
    }
    'copy' {
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        foreach ($f in $def.files) { Copy-Item -Force -Path (Join-Path $src $f) -Destination $dest }
        Done $true "copied to $dest"
    }
    '7z' {
        $z = 'C:\kfapps\tools\7zr.exe'
        if (-not (Test-Path $z)) { Done $false '7zr.exe not staged (package sevenzip)' }
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        foreach ($f in $def.files) {
            & $z x -y ('-o' + $dest) (Join-Path $src $f) 2>&1 | Out-Null
            if ($LASTEXITCODE -ne 0) { Done $false "7zr x $f failed rc=$LASTEXITCODE" }
        }
        if ($def.strip_top) { Flatten-SingleTop $dest }
        Done $true "extracted to $dest"
    }
    'msi_admin' {
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        foreach ($f in $def.files) {
            $p = Start-Process msiexec.exe -ArgumentList @('/a', ('"' + (Join-Path $src $f) + '"'), '/qn', ('TARGETDIR="' + $dest + '"')) -Wait -PassThru
            if ($p.ExitCode -ne 0) { Done $false "msiexec /a $f failed rc=$($p.ExitCode)" }
        }
        Done $true "administrative image in $dest"
    }
    'installer' {
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        $file = Join-Path $src $def.files[0]
        $argv = @()
        foreach ($a in $def.run) { $argv += ($a.Replace('{file}', $file).Replace('{dest}', $dest)) }
        $exe = $argv[0]; $rest = @($argv | Select-Object -Skip 1)
        $p = Start-Process -FilePath $exe -ArgumentList $rest -Wait -PassThru
        # installers differ in what rc 0/3010 mean; the app's own existence check is the real test
        Done ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010) "installer rc=$($p.ExitCode)"
    }
    default { Done $false "unknown stage mode $($def.mode)" }
}
