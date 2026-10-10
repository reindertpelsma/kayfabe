(Get-Date).ToString("o")
"--- uptime/boot"; (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
"--- System events since sign-in (non-info)"
Get-WinEvent -FilterHashtable @{LogName='System';StartTime=(Get-Date).AddMinutes(-9)} -ErrorAction SilentlyContinue | Where-Object {$_.Level -le 3 -or $_.ProviderName -match 'nvlddmkm|dxgkrnl|Display|LiveKernel|WER'} | Select-Object TimeCreated,ProviderName,Id,@{n='M';e={($_.Message -split "`n")[0]}} | Format-Table -AutoSize -Wrap | Out-String -Width 220
"--- LiveKernelReports"; Get-ChildItem C:\Windows\LiveKernelReports -Recurse -ErrorAction SilentlyContinue | Select FullName,Length,LastWriteTime | Format-Table -Auto | Out-String -Width 200
"--- responding"; Get-Process explorer,dwm,LogonUI,csrss,winlogon -ErrorAction SilentlyContinue | Select Name,Id,Responding,CPU | Format-Table | Out-String
"--- nvidia-smi"; & nvidia-smi --query-gpu=name,utilization.gpu,memory.used --format=csv,noheader 2>&1
"--- top cpu"; Get-Process | Sort CPU -desc | Select -First 6 Name,Id,CPU | Format-Table | Out-String
