'--- query'
& nvidia-smi.exe --query-gpu=name,memory.total,memory.used,memory.free,pstate,utilization.gpu,temperature.gpu,power.draw,clocks.gr,pci.bus_id,uuid,vbios_version,display_active --format=csv 2>&1 | Out-String
'--- -q MEMORY'
& nvidia-smi.exe -q -d MEMORY 2>&1 | Out-String
'--- Win32_VideoController'
Get-CimInstance Win32_VideoController | Where-Object { $_.Name -match 'NVIDIA' } | Select-Object Name,AdapterRAM,DriverVersion,VideoProcessor,CurrentHorizontalResolution | Format-List | Out-String
'--- dxgkrnl operational events'
Get-WinEvent -ListLog '*DxgKrnl*','*Display*','*Direct3D*','*D3D*' -ErrorAction SilentlyContinue | Where-Object RecordCount -gt 0 | Select-Object LogName,RecordCount | Format-Table -AutoSize | Out-String
