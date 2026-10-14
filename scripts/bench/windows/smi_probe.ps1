# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# smi_probe.ps1 -- read-only (2026-10-08): the goal-check nvidia-smi fields and the adapter state, one line each.
$smi = "$env:SystemRoot\System32\nvidia-smi.exe"
if (-not (Test-Path $smi)) { $smi = 'nvidia-smi.exe' }
"SMI query:"
& $smi --query-gpu=name,driver_version,memory.total,memory.used,memory.free,pstate,utilization.gpu,temperature.gpu,display_active,display_mode --format=csv 2>&1 | ForEach-Object { "SMI $_" }
"SMI exit=$LASTEXITCODE"
& $smi -q -d MEMORY 2>&1 | Select-String -Pattern 'Total|Used|Free' | ForEach-Object { "SMI mem $($_.Line.Trim())" }
Get-PnpDevice -Class Display | ForEach-Object { "SMI adapter '$($_.FriendlyName)' Status=$($_.Status) Problem=$($_.Problem) Present=$($_.Present)" }
