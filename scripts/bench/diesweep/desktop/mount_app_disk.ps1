# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Use the same inbox ISO mount path as the native Windows app-matrix reference.
param([string]$ImagePath='C:\kfmedia\kfapps.iso',
      [Parameter(Mandatory=$true)][string]$ExpectedSha256,
      [Parameter(Mandatory=$true)][long]$ExpectedSize)
$ErrorActionPreference='Stop'
$file=Get-Item -LiteralPath $ImagePath
if($file.Length -ne $ExpectedSize){throw 'app disk size mismatch'}
$hash=(Get-FileHash -LiteralPath $ImagePath -Algorithm SHA256).Hash.ToLower()
if($hash -ne $ExpectedSha256){throw 'app disk hash mismatch'}
$disk=Get-DiskImage -ImagePath $ImagePath
if(!$disk.Attached){Mount-DiskImage -ImagePath $ImagePath -StorageType ISO -Access ReadOnly | Out-Null}
if(!(Get-DiskImage -ImagePath $ImagePath).Attached){throw 'app disk mount did not attach'}
'S4_APPDISK_MOUNTED sha256='+$hash+' bytes='+$file.Length+' access=ReadOnly'
