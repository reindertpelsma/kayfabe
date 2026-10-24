# B1n DIAGNOSTIC arm on_tdr60 (owner 2026-10-24): guest registry TdrDdiDelay=60 set in a
# standard-VGA prep boot (no kf3 device), effective from the next boot. Never a product setting.
$ErrorActionPreference='Stop'
$k='HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
New-ItemProperty -LiteralPath $k -Name TdrDdiDelay -PropertyType DWord -Value 60 -Force | Out-Null
$v=(Get-ItemProperty -LiteralPath $k).TdrDdiDelay
if($v -ne 60){ throw 'TdrDdiDelay readback is not 60' }
$d=(Get-ItemProperty -LiteralPath $k).TdrDelay
$t=[DateTime]::UtcNow.ToString('o')
"PREP_TDR60_DONE TdrDdiDelay=$v TdrDelay=$d label=DIAGNOSTIC UTC=$t"
