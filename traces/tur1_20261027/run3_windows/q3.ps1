$K = 'C:\kf\tools'
function Run($name, $exe, $args_) {
  $t0 = Get-Date
  $o = & "$K\$exe" @args_ 2>&1 | Out-String
  $rc = $LASTEXITCODE
  $secs = [int]((Get-Date) - $t0).TotalSeconds
  "=== $name rc=$rc secs=$secs"
  ($o -split "`r?`n" | Where-Object { $_ -match '^(KFCUDA|KFDX|CHECK|CE rv|CUP|PROBE|BENCH)' -or $_ -match 'FAIL|RESULT' } | Select-Object -First 14) -join "`n"
}
Run 'cuda_callback' 'kf_cuda_control.exe' @('callback')
Run 'cuda_occupancy' 'kf_cuda_control.exe' @('occupancy')
Run 'dxprobe_d3d11' 'kf_dxprobe.exe' @('d3d11','--iters','200')
Run 'dxprobe_d3d12' 'kf_dxprobe.exe' @('d3d12','--iters','200')
Run 'cup2' 'cup2.exe' @()
Run 'cup3' 'cup3.exe' @()
Run 'cup8' 'cup8.exe' @()
"=== events since boot"
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=(Get-Date).AddMinutes(-20)} -ErrorAction SilentlyContinue |
  Where-Object { $_.Id -in 4101,153,141,117,1001 -or $_.ProviderName -match 'nvlddmkm' } |
  Select-Object -First 10 TimeCreated,Id,ProviderName | Format-Table | Out-String
"=== adapters"
Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,Status,ConfigManagerErrorCode | Format-List | Out-String
