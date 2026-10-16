# kfp_smi.ps1 -File out.csv -Stop stop.flag [-Every 1] : GPU util/clocks/power/pstate/temp/mem + guest CPU% at ~1 Hz until the stop flag exists.
param([string]$File, [string]$Stop, [int]$Every = 1)
"t,util,sm_mhz,mem_mhz,power_w,pstate,temp_c,mem_mib,cpu_pct" | Set-Content $File -Encoding ascii
while (-not (Test-Path $Stop)) {
  $q = (& "C:\Windows\System32\nvidia-smi.exe" --query-gpu=utilization.gpu,clocks.sm,clocks.mem,power.draw,pstate,temperature.gpu,memory.used --format=csv,noheader,nounits 2>$null) -replace " ", ""
  $c = [int](Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'").PercentProcessorTime
  "$((Get-Date).ToString("HH:mm:ss.fff")),$q,$c" | Add-Content $File -Encoding ascii
  Start-Sleep -Seconds $Every
}
