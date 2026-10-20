# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Executes the actual production setup/collector through Windows PowerShell 5.1.
param([string]$Scripts, [string]$Root)
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Force $Root | Out-Null
$pm="$Root\fixture.csv"; $trig="$Root\fixture-trig.log"
$header='Application,ProcessID,TimeInQPC'
$header | Set-Content $pm
'' | Set-Content $trig
$setup=Get-Content "$Scripts\post_boot.ps1" -Raw
$prefix=$setup.Substring(0,$setup.IndexOf('& logman stop'))
$prefix=$prefix.Replace("C:\kf\hang\pm_live.csv",$pm).Replace("C:\kf\hang\trig.log",$trig)
& ([scriptblock]::Create($prefix)) -Root $Root
'9999,Heaven.exe,7,1000' | Add-Content $pm
$source=(Get-Content "$Scripts\collect.ps1" -Raw).Replace('C:\kf\hang\pm_live.csv',$pm).Replace('C:\kf\hang\trig.log',$trig)
$payload=& ([scriptblock]::Create($source)) -Root $Root
$p=$payload | ConvertFrom-Json
$b=[Convert]::FromBase64String($p.files.'pm_live.csv')
$m=New-Object IO.MemoryStream(,$b)
$z=New-Object IO.Compression.GZipStream($m,[IO.Compression.CompressionMode]::Decompress)
$reader=New-Object IO.StreamReader($z)
$csv=$reader.ReadToEnd(); $reader.Dispose(); $m.Dispose()
$decorated=Get-Content $pm -TotalCount 1
$scalar=$null
if(Get-Command ConvertTo-N1TransportValue -ErrorAction SilentlyContinue) {
    $scalar=ConvertTo-N1TransportValue @{label=$decorated; count=[long]17; at=[datetime]'2026-10-20T01:00:00Z'; items=@($decorated)}
}
@{powershell_version=$PSVersionTable.PSVersion.ToString(); meta=$p.meta; csv=$csv; payload=$p; guarded=$scalar} | ConvertTo-Json -Depth 20 -Compress
