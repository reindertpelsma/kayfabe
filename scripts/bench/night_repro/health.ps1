# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([string]$Root)
$ErrorActionPreference='Stop'
. "$PSScriptRoot\transport.ps1"
$m=Get-Content "$Root\start.json" -Raw | ConvertFrom-Json
$start=[datetime]::Parse($m.start_utc).ToUniversalTime()
$events=@(Get-WinEvent -FilterHashtable @{LogName='System'; ProviderName='nvlddmkm'; Id=153; StartTime=$start.ToLocalTime()} -ErrorAction SilentlyContinue | ForEach-Object {
    @{provider=[string]$_.ProviderName; id=[int]$_.Id; utc=$_.TimeCreated.ToUniversalTime().ToString('o'); elapsed_s=($_.TimeCreated.ToUniversalTime()-$start).TotalSeconds}
})
$losses=@(Get-ChildItem "$Root\*.stderr.txt" -ErrorAction SilentlyContinue | Select-String -Pattern 'device lost|device removed|device hung|DXGI_ERROR_DEVICE' | ForEach-Object { @{file=[string]$_.Path; line=[string]$_.Line} })
ConvertTo-N1Json @{events=$events; device_losses=$losses; qpc=[Diagnostics.Stopwatch]::GetTimestamp()}
