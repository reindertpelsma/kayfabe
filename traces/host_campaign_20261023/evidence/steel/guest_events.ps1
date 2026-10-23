$b=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$e=@(Get-WinEvent -FilterHashtable @{LogName="System";StartTime=$b} -ErrorAction SilentlyContinue | ?{ ($_.Id -eq 153 -and $_.ProviderName -eq "nvlddmkm") -or ($_.Id -eq 4101 -and $_.ProviderName -eq "Display") -or ($_.Id -eq 141 -and $_.ProviderName -match "Display|nvlddmkm|Kernel") -or $_.ProviderName -eq "nvlddmkm" })
"BOOT=$b nvlddmkm_or_tdr_events=$($e.Count) 153=$(($e|?{$_.Id -eq 153}).Count) 4101=$(($e|?{$_.Id -eq 4101}).Count) 141=$(($e|?{$_.Id -eq 141}).Count)"
