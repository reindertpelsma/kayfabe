# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# open_events.ps1 -SinceUtc <ISO8601 Z> -- the crash and reset events since a time, as one JSON array on stdout (thin glue for gl_soak.py; no logic).
# Application Error (1000) and Windows Error Reporting (1001), the NVIDIA kernel driver's TDR recovery (System, nvlddmkm 153) and Display 4101.
param([Parameter(Mandatory = $true)][string]$SinceUtc)
$ErrorActionPreference = 'Continue'
$since = ([datetime]::Parse($SinceUtc, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal)).ToLocalTime()
$rows = @()
$rows += @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 1000, 1001; StartTime = $since } -ErrorAction SilentlyContinue)
$rows += @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $since } -ErrorAction SilentlyContinue | Where-Object { ($_.ProviderName -eq 'nvlddmkm' -and $_.Id -eq 153) -or ($_.ProviderName -eq 'Display' -and $_.Id -eq 4101) })
$out = @($rows | ForEach-Object { [pscustomobject]@{ Id = [int]$_.Id; Provider = [string]$_.ProviderName; Message = ([string]$_.Message -replace '[\r\n]+', ' ') } })
ConvertTo-Json -InputObject $out -Compress
