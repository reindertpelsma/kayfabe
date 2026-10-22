# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedSha256,
      [Parameter(Mandatory=$true)][long]$ExpectedSize)
$ErrorActionPreference='Stop'
$zip='C:\kfmedia\cuda_nvrtc.zip'
if((Get-Item -LiteralPath $zip).Length -ne $ExpectedSize){throw 'NVRTC supplement size mismatch'}
$sha=(Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLower()
if($sha -ne $ExpectedSha256){throw 'NVRTC supplement checksum mismatch'}
$dest='C:\kfapps\cuda_nvrtc'; $marker='C:\kfapps\.staged\cuda_nvrtc.json'
Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $dest -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $dest | Out-Null
& tar.exe -xf $zip -C $dest
if($LASTEXITCODE -ne 0){throw 'NVRTC supplement extraction failed'}
$items=@(Get-ChildItem -LiteralPath $dest -Force)
if($items.Count -eq 1 -and $items[0].PSIsContainer){
 $top=$items[0].FullName
 Get-ChildItem -LiteralPath $top -Force | Move-Item -Destination $dest -Force
 Remove-Item -LiteralPath $top -Recurse -Force
}
foreach($dll in 'nvrtc64_120_0.dll','nvrtc-builtins64_129.dll'){
 if(!(Test-Path -LiteralPath (Join-Path $dest ('bin\'+$dll)) -PathType Leaf)){throw 'NVRTC runtime DLL missing'}
}
$r=@{pkg='cuda_nvrtc';ok=$true;mode='pinned-supplement';sha256=$sha;bytes=$ExpectedSize;detail='verified manifest-pinned supplementary archive'}
New-Item -ItemType Directory -Force -Path 'C:\kfapps\.staged' | Out-Null
$r|ConvertTo-Json -Compress|Set-Content -LiteralPath ($marker+'.tmp') -Encoding UTF8
Move-Item -LiteralPath ($marker+'.tmp') -Destination $marker -Force
'S4_NVRTC_STAGED '+($r|ConvertTo-Json -Compress)
