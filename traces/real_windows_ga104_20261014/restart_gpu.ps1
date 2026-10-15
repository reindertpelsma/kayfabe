# Restarts the NVIDIA display adapter (pnputil /restart-device). The device instance id carries the PCI
# subsystem id (board data), so it is never printed: only pnputil's exit code and the elapsed time are.
$d = Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' } | Select-Object -First 1
if (-not $d) { throw 'no NVIDIA display device' }
$t = [Diagnostics.Stopwatch]::StartNew()
$o = & pnputil.exe /restart-device $d.InstanceId 2>&1
"pnputil rc=$LASTEXITCODE elapsed_ms=$($t.ElapsedMilliseconds) restarted=" + [bool]($o -match 'restarted')
