"=== nvidia-smi"
& 'C:\Windows\System32\nvidia-smi.exe' 2>&1 | Out-String
"=== nvidia-smi -q (selected)"
& 'C:\Windows\System32\nvidia-smi.exe' -q 2>&1 | Select-String -Pattern 'Product Name|Product Architecture|Driver Version|CUDA Version|GSP Firmware|VBIOS|Bus Id|Total  |Used  |Display|Performance State|Xid|Retired' | Out-String
"=== dxdiag-ish: video controller"
Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,Status,ConfigManagerErrorCode,AdapterRAM | Format-List | Out-String
"=== recent System events (nvlddmkm / display / TDR)"
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=(Get-Date).AddMinutes(-30)} -ErrorAction SilentlyContinue |
  Where-Object { $_.ProviderName -match 'nvlddmkm|Display|dxgkrnl|BugCheck|Kernel-Power' -or $_.Id -in 4101,41,1001,153 } |
  Select-Object -First 20 TimeCreated,Id,ProviderName,@{n='M';e={($_.Message -replace '\s+',' ').Substring(0,[Math]::Min(200,$_.Message.Length))}} | Format-List | Out-String
