# smi_sampler.ps1 -Dir <outdir> [-Every 2] : CSV of guest time, GPU memory used (MiB), GPU util, free RAM (MiB) until <Dir>\stop.flag exists
param([string]$Dir, [int]$Every = 1)
$f = "$Dir\smi.csv"; "t,gpu_mem_mib,gpu_util,free_ram_mib" | Set-Content $f
while (-not (Test-Path "$Dir\stop.flag")) {
  $q = (& 'C:\Windows\System32\nvidia-smi.exe' --query-gpu=memory.used,utilization.gpu --format=csv,noheader,nounits 2>$null) -replace ' ', ''
  $r = [int]((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024)
  "$((Get-Date).ToString('HH:mm:ss')),$q,$r" | Add-Content $f
  Start-Sleep -Seconds $Every
}
