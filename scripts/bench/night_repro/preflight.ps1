# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([string]$Scripts,[string]$Root)
$ErrorActionPreference='Stop'
$green=& "$Scripts\test_transport.ps1" -Scripts $Scripts -Root "$Root\green"
'PREFLIGHT_GREEN='+[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$green))
$path="$Scripts\transport.ps1"; $original=[IO.File]::ReadAllText($path)
$old=@'
if($Value -is [string]) { return ('{0}' -f $Value) }
'@
$new=@'
if($Value -is [string]) { if(([string]$Value).StartsWith('Application,')) { return [pscustomobject]@{value=('{0}' -f $Value)} }; return ('{0}' -f $Value) }
'@
if(-not $original.Contains($old.Trim())) { throw 'Mutation seam absent' }
try {
    [IO.File]::WriteAllText($path,$original.Replace($old.Trim(),$new.Trim()))
    $mutant=& "$Scripts\test_transport.ps1" -Scripts $Scripts -Root "$Root\mutant"
    'PREFLIGHT_MUTANT='+[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$mutant))
} finally { [IO.File]::WriteAllText($path,$original) }
'PREFLIGHT_RESTORED'
